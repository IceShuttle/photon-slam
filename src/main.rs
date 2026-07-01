use std::sync::Arc;

use image::ImageReader;
use vulkano::VulkanLibrary;
use vulkano::buffer::{Buffer, BufferCreateInfo, BufferUsage};
use vulkano::command_buffer::{
    AutoCommandBufferBuilder, CommandBufferUsage, CopyBufferToImageInfo,
    allocator::StandardCommandBufferAllocator,
};
use vulkano::device::{Device, DeviceCreateInfo, DeviceExtensions, QueueCreateInfo, QueueFlags};
use vulkano::format::Format;
use vulkano::image::{Image, ImageCreateInfo, ImageType, ImageUsage, view::ImageView};
use vulkano::instance::{Instance, InstanceCreateFlags, InstanceCreateInfo};
use vulkano::memory::allocator::{AllocationCreateInfo, MemoryTypeFilter, StandardMemoryAllocator};
use vulkano::swapchain::Surface;
use vulkano::sync::{self, GpuFuture};
use winit::event_loop::{self, EventLoop};
mod window;

fn main() {
    println!("Starting...");

    let library = VulkanLibrary::new().expect("no local Vulkan library/DLL");
    let event_loop = EventLoop::new().unwrap();

    let required_extensions = Surface::required_extensions(&event_loop).unwrap();
    let device_extensions = DeviceExtensions {
        khr_swapchain: true,
        ..Default::default()
    };

    let instance = Instance::new(
        library,
        InstanceCreateInfo {
            flags: InstanceCreateFlags::ENUMERATE_PORTABILITY,
            enabled_extensions: required_extensions,
            ..Default::default()
        },
    )
    .expect("failed to create instance");

    let physical_device = instance
        .enumerate_physical_devices()
        .expect("could not enumerate devices")
        .next()
        .expect("no devices available");

    photon_slam::print_info(&physical_device);

    let queue_family_index = physical_device
        .queue_family_properties()
        .iter()
        .position(|qfp| {
            qfp.queue_flags
                .contains(QueueFlags::GRAPHICS & QueueFlags::COMPUTE)
        })
        .expect("couldn't find a graphics queue family") as u32;

    let (device, mut queues) = Device::new(
        physical_device.clone(),
        DeviceCreateInfo {
            queue_create_infos: vec![QueueCreateInfo {
                queue_family_index,
                ..Default::default()
            }],
            enabled_extensions: device_extensions,
            ..Default::default()
        },
    )
    .expect("failed to create device");

    let queue = queues.next().unwrap();

    let memory_allocator = Arc::new(StandardMemoryAllocator::new_default(device.clone()));

    let cmd_buff_allocator = Arc::new(StandardCommandBufferAllocator::new(
        device.clone(),
        Default::default(),
    ));

    println!("Vulkan Initialized");

    let mut uploads = AutoCommandBufferBuilder::primary(
        cmd_buff_allocator.clone(),
        queue_family_index,
        CommandBufferUsage::OneTimeSubmit,
    )
    .unwrap();

    let texture = {
        let img = ImageReader::open("cat.jpg").unwrap().decode().unwrap();
        let upload_buffer = Buffer::from_iter(
            memory_allocator.clone(),
            BufferCreateInfo {
                usage: BufferUsage::TRANSFER_SRC,
                ..Default::default()
            },
            AllocationCreateInfo {
                memory_type_filter: MemoryTypeFilter::PREFER_HOST
                    | MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
                ..Default::default()
            },
            img.to_rgba8().into_raw(),
        )
        .unwrap();

        let image = Image::new(
            memory_allocator.clone(),
            ImageCreateInfo {
                image_type: ImageType::Dim2d,
                format: Format::R8G8B8A8_SRGB,
                extent: [img.width(), img.height(), 1],
                usage: ImageUsage::TRANSFER_DST | ImageUsage::TRANSFER_SRC | ImageUsage::SAMPLED,
                ..Default::default()
            },
            AllocationCreateInfo {
                memory_type_filter: MemoryTypeFilter::PREFER_DEVICE,
                ..Default::default()
            },
        )
        .unwrap();

        uploads
            .copy_buffer_to_image(CopyBufferToImageInfo::buffer_image(
                upload_buffer,
                image.clone(),
            ))
            .unwrap();
        ImageView::new_default(image).unwrap()
    };
    {
        let cmd_buff = uploads.build().unwrap(); // Uploads is builded here
        sync::now(device.clone())
            .then_execute(queue.clone(), cmd_buff)
            .unwrap()
            .flush()
            .unwrap();
        println!("Image uploaded");
    }

    let mut app = window::App::new(
        instance,
        physical_device,
        device,
        queue,
        texture,
        None,
        None,
        cmd_buff_allocator,
    );
    event_loop.set_control_flow(event_loop::ControlFlow::Poll);
    event_loop.run_app(&mut app).unwrap();
}
