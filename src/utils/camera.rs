//! V4L2 camera capture → Vulkan image via CPU staging copy.
//!
//! Pipeline:
//! 1. V4L2 mmap buffer capture (kernel writes DMA into mapped pages)
//! 2. CPU pixel conversion to RGBA (MJPEG decode, YUV→RGB, etc.)
//! 3. CPU copy from conversion buffer → host-visible staging buffer
//! 4. `vkCmdCopyBufferToImage` → GPU-optimal image
//!
//! This avoids the fragile and crash-prone DMA-BUF/VK_EXTERNAL_MEMORY path
//! (which caused segfaults on Intel Mesa with `DrmFormatModifier`).
//! The extra CPU work (decode + copy) is invisible compared to the camera
//! frame interval (~33 ms at 30 fps).

use crate::vulkan::context::VulkanContext;
use anyhow::{Context, Result};
use std::os::fd::FromRawFd;
use std::sync::Arc;
use v4l::buffer::Type;
use v4l::control::{Control, Value};
use v4l::io::traits::{CaptureStream, Stream};
use v4l::v4l_sys::{v4l2_buf_type_V4L2_BUF_TYPE_VIDEO_CAPTURE, v4l2_exportbuffer};
use v4l::v4l2::ioctl;
use v4l::v4l2::vidioc::VIDIOC_EXPBUF;
use v4l::video::Capture;
use v4l::{FourCC, prelude::*};
use vulkano::device::Device;
use vulkano::format::Format;
use vulkano::image::sys::RawImage;
use vulkano::image::{Image, ImageCreateInfo, ImageTiling, ImageType, ImageUsage};
use vulkano::memory::{
    DeviceMemory, ExternalMemoryHandleType, ExternalMemoryHandleTypes, MemoryAllocateInfo,
    MemoryImportInfo, MemoryRequirements, ResourceMemory,
};

/// Number of V4L2 mmap buffers to request from the driver.
const BUFFER_COUNT: usize = 4;

/// Camera capture config
pub struct CameraConfig {
    pub width: u32,
    pub height: u32,
}

impl Default for CameraConfig {
    fn default() -> Self {
        Self {
            width: 640,
            height: 480,
        }
    }
}

/// Implements
pub struct CameraCapture<'a> {
    /// The V4L2 pixel format negotiated by the driver.
    stream: MmapStream<'a>,
    gpu_images: [Arc<Image>; BUFFER_COUNT],
    pub pixel_format: v4l::FourCC,
    pub width: u32,
    pub height: u32,
}

impl CameraCapture<'_> {
    /// Open `/dev/video0`, negotiate format at the requested resolution,
    /// mmap buffers, and pre-allocate the GPU image and queues them
    pub fn new(ctx: &VulkanContext, config: &CameraConfig) -> Result<Self> {
        let video_device = v4l::Device::new(0).context("failed to open /dev/video0")?;
        let device_fd = video_device.handle().fd();
        tracing::info!("Video device created");

        // Setting up params
        let mut params = video_device.params()?;
        params.interval = v4l::Fraction {
            numerator: 1,
            denominator: 30,
        };

        // Setting auto exposure to manual(1)
        let auto_exp_priority = Control {
            id: 0x009A0901,
            value: Value::Integer(1),
        };
        video_device.set_control(auto_exp_priority)?;

        video_device.set_params(&params)?;
        params = video_device.params()?;
        tracing::debug!("Camera Params:{params}");
        let avail_formats = video_device.enum_formats();
        tracing::debug!("Available formats {:?}", avail_formats);

        // Negotiate format — Request YUYV (will implement NV12 later).
        let mut fmt = video_device.format()?;
        fmt.width = config.width;
        fmt.height = config.height;
        fmt.fourcc = FourCC::new(b"YUYV");
        let negotiated = video_device.set_format(&fmt)?;

        tracing::info!(
            "V4L2: {}x{} fourcc={} stride={} size={}",
            negotiated.width,
            negotiated.height,
            negotiated.fourcc,
            negotiated.stride,
            negotiated.size,
        );

        let pixel_format = negotiated.fourcc;
        let width = negotiated.width;
        let height = negotiated.height;

        let mut stream =
            MmapStream::with_buffers(&video_device, Type::VideoCapture, BUFFER_COUNT as u32)
                .context("failed to create mmap buffer stream")?;

        let gpu_images: Vec<Arc<Image>> = (0..BUFFER_COUNT)
            .map(|i| -> Result<Arc<Image>> {
                stream.queue(i as usize)?;
                let mut exp_buf: v4l2_exportbuffer = unsafe { std::mem::zeroed() };
                exp_buf.type_ = v4l2_buf_type_V4L2_BUF_TYPE_VIDEO_CAPTURE;
                exp_buf.index = i as u32;
                unsafe {
                    ioctl(
                        device_fd,
                        VIDIOC_EXPBUF,
                        &mut exp_buf as *mut _ as *mut std::ffi::c_void,
                    )?;
                }
                tracing::info!("Buffer {i} exported");

                let raw_image = RawImage::new(
                    ctx.device.clone(),
                    ImageCreateInfo {
                        image_type: ImageType::Dim2d,
                        format: Format::G8B8G8R8_422_UNORM,
                        extent: [width, height, 1],
                        usage: ImageUsage::TRANSFER_SRC,
                        tiling: ImageTiling::Linear,
                        external_memory_handle_types: ExternalMemoryHandleTypes::DMA_BUF,
                        ..Default::default()
                    },
                )?;
                let memory_requirements = raw_image.memory_requirements()[0];

                let device_memory = unsafe {
                    DeviceMemory::import(
                        ctx.device.clone(),
                        MemoryAllocateInfo {
                            allocation_size: memory_requirements.layout.size(),
                            memory_type_index: find_memory_type_index(
                                &ctx.device,
                                memory_requirements,
                            )?,
                            dedicated_allocation: Some(
                                vulkano::memory::DedicatedAllocation::Image(&raw_image),
                            ),
                            ..Default::default()
                        },
                        MemoryImportInfo::Fd {
                            handle_type: ExternalMemoryHandleType::DmaBuf,
                            file: std::fs::File::from_raw_fd(exp_buf.fd),
                        },
                    )
                }?;
                let resource_memory = ResourceMemory::new_dedicated(device_memory);
                let img = raw_image
                    .bind_memory([resource_memory])
                    .map_err(|(err, _, _)| err)?;

                tracing::debug!("Buffer {i} converted to image");
                Ok(Arc::new(img))
            })
            .map(|x| x.unwrap())
            .collect();

        let gpu_images: [Arc<Image>; BUFFER_COUNT] = gpu_images.try_into().unwrap();

        stream.start()?;

        tracing::info!("CameraCapture instantiated");

        Ok(Self {
            pixel_format,
            stream,
            gpu_images,
            width,
            height,
        })
    }

    //// Dequeues the next filled V4L2 buffer and returns its index plus the
    /// index of the Vulkan image already bound (zero-copy) to that buffer's dma-buf.
    pub fn capture(&mut self) -> Result<(usize, Arc<Image>)> {
        let index = self
            .stream
            .dequeue()
            .context("failed to dequeue V4L2 buffer")?;
        let img = self.gpu_images[index].clone();
        Ok((index, img))
    }

    /// Hands a buffer back to the V4L2 driver for reuse. Only call this
    /// after the GPU has finished with `gpu_images[index]`.
    pub fn release(&mut self, index: usize) -> Result<()> {
        self.stream
            .queue(index)
            .context("failed to requeue V4L2 buffer")
    }
}

impl Drop for CameraCapture<'_> {
    fn drop(&mut self) {
        self.stream.stop().unwrap();
    }
}

fn find_memory_type_index(device: &Device, requirements: MemoryRequirements) -> Result<u32> {
    let memory_properties = device.physical_device().memory_properties();

    memory_properties
        .memory_types
        .iter()
        .enumerate()
        .position(|(idx, _)| (requirements.memory_type_bits & (1 << idx)) != 0)
        .map(|idx| idx as u32)
        .context("No matching memory type found for DMA-BUF import")
}
