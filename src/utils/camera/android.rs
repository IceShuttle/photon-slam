//! Camera2 NDK capture → Vulkan image via CPU repack.
//!
//! Pipeline:
//! 1. `ACameraDevice` streams into an `AImageReader` configured for the
//!    universally-supported `YUV_420_888` format (Camera2 has no portable
//!    equivalent of V4L2's packed YUYV).
//! 2. Each delivered `AImage`'s Y/U/V planes are repacked on the CPU into the
//!    same "YUYV-as-RGBA" byte layout `linux.rs` produces (Y0→R, U→G, Y1→B,
//!    V→A), so the existing `YuvyToR8Pass` shader needs no android-specific
//!    branch.
//! 3. The repacked bytes are written straight into a host-visible, linearly
//!    tiled `vulkano::image::Image` that is mapped once at startup — no
//!    per-frame command buffer/queue is needed (`new()` only receives a
//!    `Device`, matching `V4lCapture::new`'s signature).
//! 4. `release()` is a no-op: unlike the V4L2 mmap buffers, the source
//!    `AImage` is freed right after its bytes are copied in `capture()`, so
//!    there is no driver-owned buffer to hand back.

use super::CameraConfig;
use anyhow::{bail, Context, Result};
use ndk::{
    media::image_reader::{AcquireResult, ImageFormat, ImageReader},
    native_window::NativeWindow,
};
use ndk_sys as ffi;
use std::{
    ffi::c_void,
    marker::PhantomData,
    ptr,
    sync::{
        mpsc::{self, Receiver},
        Arc,
    },
    time::Duration,
};
use vulkano::{
    device::Device,
    format::Format,
    image::{sys::RawImage, Image, ImageAspect, ImageCreateInfo, ImageTiling, ImageType, ImageUsage},
    memory::{
        DedicatedAllocation, DeviceMemory, MemoryAllocateInfo, MemoryMapInfo, MemoryPropertyFlags,
        MemoryRequirements, ResourceMemory,
    },
};

// Link the Camera2 NDK system library (`libcamera2ndk.so`, API 24+).
// `ndk-sys` declares the `ACamera*` symbols but never emits the link
// directive, so without this the final APK link fails with undefined
// symbols.
#[link(name = "camera2ndk")]
unsafe extern "C" {}

/// Number of `AImageReader` buffers / GPU staging images to round-robin over.
const BUFFER_COUNT: usize = 4;

/// Camera2 NDK capture, producing GPU-visible packed-YUYV frames.
pub struct AndroidCam<'a> {
    manager: *mut ffi::ACameraManager,
    device: *mut ffi::ACameraDevice,
    session: *mut ffi::ACameraCaptureSession,
    request: *mut ffi::ACaptureRequest,
    output_target: *mut ffi::ACameraOutputTarget,
    session_output: *mut ffi::ACaptureSessionOutput,
    output_container: *mut ffi::ACaptureSessionOutputContainer,
    // Kept alive for the lifetime of the session; camera APIs above hold
    // their own acquired references to the same underlying surface.
    _window: NativeWindow,
    image_reader: ImageReader,
    frame_rx: Receiver<()>,
    gpu_images: [Arc<Image>; BUFFER_COUNT],
    /// Pointers into each `gpu_images` entry's mapped, host-visible memory.
    mapped_ptrs: [*mut u8; BUFFER_COUNT],
    next_index: usize,
    config: CameraConfig,
    _marker: PhantomData<&'a ()>,
}

impl AndroidCam<'_> {
    /// Opens the first available camera, starts a `YUV_420_888` preview
    /// stream at the requested resolution, and pre-allocates the GPU images
    /// that captured frames are repacked into.
    pub fn new(vk_device: Arc<Device>, config: &CameraConfig) -> Result<Self> {
        let width = config.width;
        let height = config.height;
        let hz = config.hz.max(1);

        // 1. Image reader + listener: signals `capture()` whenever the
        //    camera has delivered a new frame.
        let mut image_reader = ImageReader::new(
            width as i32,
            height as i32,
            ImageFormat::YUV_420_888,
            BUFFER_COUNT as i32,
        )
        .context("failed to create AImageReader")?;
        let (frame_tx, frame_rx) = mpsc::channel::<()>();
        image_reader
            .set_image_listener(Box::new(move |_reader| {
                let _ = frame_tx.send(());
            }))
            .context("failed to set AImageReader image listener")?;
        let window = image_reader
            .window()
            .context("failed to get ANativeWindow from AImageReader")?;

        // 2. Open the first camera the manager reports.
        let manager = unsafe { ffi::ACameraManager_create() };
        if manager.is_null() {
            bail!("ACameraManager_create returned null");
        }

        let mut id_list: *mut ffi::ACameraIdList = ptr::null_mut();
        check(
            unsafe { ffi::ACameraManager_getCameraIdList(manager, &mut id_list) },
            "ACameraManager_getCameraIdList",
        )
        .inspect_err(|_| unsafe { ffi::ACameraManager_delete(manager) })?;
        if unsafe { (*id_list).numCameras } == 0 {
            unsafe {
                ffi::ACameraManager_deleteCameraIdList(id_list);
                ffi::ACameraManager_delete(manager);
            }
            bail!("no cameras reported by ACameraManager");
        }
        let camera_id = unsafe { *(*id_list).cameraIds };

        let mut device_callbacks = ffi::ACameraDevice_StateCallbacks {
            context: ptr::null_mut(),
            onDisconnected: Some(on_device_disconnected),
            onError: Some(on_device_error),
        };
        let mut device: *mut ffi::ACameraDevice = ptr::null_mut();
        let open_status = unsafe {
            ffi::ACameraManager_openCamera(manager, camera_id, &mut device_callbacks, &mut device)
        };
        unsafe { ffi::ACameraManager_deleteCameraIdList(id_list) };
        check(open_status, "ACameraManager_openCamera").inspect_err(|_| unsafe {
            ffi::ACameraManager_delete(manager)
        })?;

        // From here on, tear down manager+device on any error via a guard.
        let result = Self::finish_setup(
            vk_device, config, image_reader, frame_rx, window, manager, device, hz,
        );
        if result.is_err() {
            unsafe {
                ffi::ACameraDevice_close(device);
                ffi::ACameraManager_delete(manager);
            }
        }
        result
    }

    /// Builds the capture request/session and the GPU staging images. Split
    /// out of `new()` so the open camera manager/device can be torn down
    /// uniformly on any failure in this half of setup.
    fn finish_setup(
        vk_device: Arc<Device>,
        config: &CameraConfig,
        image_reader: ImageReader,
        frame_rx: Receiver<()>,
        window: NativeWindow,
        manager: *mut ffi::ACameraManager,
        device: *mut ffi::ACameraDevice,
        hz: u32,
    ) -> Result<Self> {
        let mut request: *mut ffi::ACaptureRequest = ptr::null_mut();
        check(
            unsafe {
                ffi::ACameraDevice_createCaptureRequest(
                    device,
                    ffi::ACameraDevice_request_template::TEMPLATE_PREVIEW,
                    &mut request,
                )
            },
            "ACameraDevice_createCaptureRequest",
        )?;

        let fps_range = [hz as i32, hz as i32];
        check(
            unsafe {
                ffi::ACaptureRequest_setEntry_i32(
                    request,
                    ffi::acamera_metadata_tag::ACAMERA_CONTROL_AE_TARGET_FPS_RANGE.0,
                    2,
                    fps_range.as_ptr(),
                )
            },
            "ACaptureRequest_setEntry_i32(AE_TARGET_FPS_RANGE)",
        )?;

        let mut output_target: *mut ffi::ACameraOutputTarget = ptr::null_mut();
        check(
            unsafe { ffi::ACameraOutputTarget_create(window.ptr().as_ptr(), &mut output_target) },
            "ACameraOutputTarget_create",
        )?;
        check(
            unsafe { ffi::ACaptureRequest_addTarget(request, output_target) },
            "ACaptureRequest_addTarget",
        )?;

        let mut session_output: *mut ffi::ACaptureSessionOutput = ptr::null_mut();
        check(
            unsafe { ffi::ACaptureSessionOutput_create(window.ptr().as_ptr(), &mut session_output) },
            "ACaptureSessionOutput_create",
        )?;

        let mut output_container: *mut ffi::ACaptureSessionOutputContainer = ptr::null_mut();
        check(
            unsafe { ffi::ACaptureSessionOutputContainer_create(&mut output_container) },
            "ACaptureSessionOutputContainer_create",
        )?;
        check(
            unsafe { ffi::ACaptureSessionOutputContainer_add(output_container, session_output) },
            "ACaptureSessionOutputContainer_add",
        )?;

        let session_callbacks = ffi::ACameraCaptureSession_stateCallbacks {
            context: ptr::null_mut(),
            onClosed: None,
            onReady: None,
            onActive: None,
        };
        let mut session: *mut ffi::ACameraCaptureSession = ptr::null_mut();
        check(
            unsafe {
                ffi::ACameraDevice_createCaptureSession(
                    device,
                    output_container,
                    &session_callbacks,
                    &mut session,
                )
            },
            "ACameraDevice_createCaptureSession",
        )?;

        let mut repeating_request = request;
        check(
            unsafe {
                ffi::ACameraCaptureSession_setRepeatingRequest(
                    session,
                    ptr::null_mut(),
                    1,
                    &mut repeating_request,
                    ptr::null_mut(),
                )
            },
            "ACameraCaptureSession_setRepeatingRequest",
        )?;

        // Pre-allocate the GPU-visible, host-mapped images frames are
        // repacked into. Plain (non-imported) linear-tiling memory — no
        // queue is available here to drive a buffer→image copy command.
        let extent = [config.width / 2, config.height, 1];
        let mut gpu_images: Vec<Arc<Image>> = Vec::with_capacity(BUFFER_COUNT);
        let mut mapped_ptrs: Vec<*mut u8> = Vec::with_capacity(BUFFER_COUNT);
        for i in 0..BUFFER_COUNT {
            let raw_image = RawImage::new(
                vk_device.clone(),
                ImageCreateInfo {
                    image_type: ImageType::Dim2d,
                    format: Format::R8G8B8A8_UNORM,
                    extent,
                    usage: ImageUsage::TRANSFER_SRC | ImageUsage::STORAGE,
                    tiling: ImageTiling::Linear,
                    ..Default::default()
                },
            )?;
            let requirements = raw_image.memory_requirements()[0];
            let memory_type_index = find_host_visible_memory_type_index(&vk_device, requirements)?;

            let mut device_memory = DeviceMemory::allocate(
                vk_device.clone(),
                MemoryAllocateInfo {
                    allocation_size: requirements.layout.size(),
                    memory_type_index,
                    dedicated_allocation: Some(DedicatedAllocation::Image(&raw_image)),
                    ..Default::default()
                },
            )?;
            device_memory.map(MemoryMapInfo {
                offset: 0,
                size: requirements.layout.size(),
                ..Default::default()
            })?;
            let ptr = device_memory
                .mapping_state()
                .context("camera staging image memory failed to map")?
                .ptr()
                .as_ptr()
                .cast::<u8>();

            let resource_memory = ResourceMemory::new_dedicated(device_memory);
            let image = raw_image
                .bind_memory([resource_memory])
                .map_err(|(err, _, _)| err)?;

            gpu_images.push(Arc::new(image));
            mapped_ptrs.push(ptr);
            tracing::debug!("Camera staging image {i} allocated and mapped");
        }
        let gpu_images: [Arc<Image>; BUFFER_COUNT] = gpu_images.try_into().unwrap();
        let mapped_ptrs: [*mut u8; BUFFER_COUNT] = mapped_ptrs.try_into().unwrap();

        tracing::info!(
            "Camera2: {}x{} YUV_420_888 @ {}hz",
            config.width,
            config.height,
            hz
        );

        Ok(Self {
            manager,
            device,
            session,
            request,
            output_target,
            session_output,
            output_container,
            _window: window,
            image_reader,
            frame_rx,
            gpu_images,
            mapped_ptrs,
            next_index: 0,
            config: CameraConfig {
                pixel_format: Format::B8G8R8G8_422_UNORM,
                width: config.width,
                height: config.height,
                hz,
            },
            _marker: PhantomData,
        })
    }

    /// Waits for the next camera frame and repacks its Y/U/V planes into the
    /// next GPU staging image, returning its index plus the bound image.
    pub fn capture(&mut self) -> Result<(usize, Arc<Image>)> {
        self.frame_rx
            .recv_timeout(Duration::from_secs(2))
            .context("timed out waiting for a camera frame")?;
        // Coalesce any backlog — only the freshest frame matters.
        while self.frame_rx.try_recv().is_ok() {}

        let image = match self
            .image_reader
            .acquire_latest_image()
            .context("failed to acquire camera image")?
        {
            AcquireResult::Image(image) => image,
            _ => bail!("camera image reader had no buffer available"),
        };

        let width = self.config.width as usize;
        let height = self.config.height as usize;
        let y = image.plane_data(0)?;
        let u = image.plane_data(1)?;
        let v = image.plane_data(2)?;
        let y_stride = image.plane_row_stride(0)? as usize;
        let u_stride = image.plane_row_stride(1)? as usize;
        let v_stride = image.plane_row_stride(2)? as usize;
        let u_pixel_stride = image.plane_pixel_stride(1)? as usize;
        let v_pixel_stride = image.plane_pixel_stride(2)? as usize;

        let index = self.next_index;
        self.next_index = (self.next_index + 1) % BUFFER_COUNT;
        let gpu_image = self.gpu_images[index].clone();
        let layout = gpu_image.subresource_layout(ImageAspect::Color, 0, 0)?;
        let dst = self.mapped_ptrs[index];

        // YUV_420_888 → packed YUYV-as-RGBA: Y0→R, U→G, Y1→B, V→A per pixel
        // pair, matching what `YuvyToR8Pass` expects from `linux.rs`.
        for row in 0..height {
            let y_row = &y[row * y_stride..];
            let u_row = &u[(row / 2) * u_stride..];
            let v_row = &v[(row / 2) * v_stride..];
            // SAFETY: `dst` points at `layout.size` mapped, host-coherent
            // bytes owned solely by `gpu_images[index]`; no other reference
            // to it is alive while we hold `&mut self`.
            unsafe {
                let dst_row = dst.add(layout.offset as usize + row * layout.row_pitch as usize);
                for pair in 0..width / 2 {
                    let texel = dst_row.add(pair * 4);
                    *texel = y_row[pair * 2];
                    *texel.add(1) = u_row[pair * u_pixel_stride];
                    *texel.add(2) = y_row[pair * 2 + 1];
                    *texel.add(3) = v_row[pair * v_pixel_stride];
                }
            }
        }

        Ok((index, gpu_image))
    }

    /// No-op: the source `AImage` is freed at the end of `capture()`, so
    /// there is no driver-owned buffer left to hand back.
    pub fn release(&mut self, _index: usize) -> Result<()> {
        Ok(())
    }

    /// Returns the Camera config
    pub fn config(&self) -> CameraConfig {
        self.config
    }
}

impl Drop for AndroidCam<'_> {
    fn drop(&mut self) {
        unsafe {
            ffi::ACameraCaptureSession_stopRepeating(self.session);
            ffi::ACameraCaptureSession_close(self.session);
            ffi::ACaptureRequest_free(self.request);
            ffi::ACameraOutputTarget_free(self.output_target);
            ffi::ACaptureSessionOutput_free(self.session_output);
            ffi::ACaptureSessionOutputContainer_free(self.output_container);
            ffi::ACameraDevice_close(self.device);
            ffi::ACameraManager_delete(self.manager);
        }
    }
}

unsafe extern "C" fn on_device_disconnected(_context: *mut c_void, _device: *mut ffi::ACameraDevice) {
    tracing::warn!("Camera device disconnected");
}

unsafe extern "C" fn on_device_error(
    _context: *mut c_void,
    _device: *mut ffi::ACameraDevice,
    error: std::os::raw::c_int,
) {
    tracing::error!("Camera device error: {error}");
}

fn check(status: ffi::camera_status_t, what: &str) -> Result<()> {
    if status == ffi::camera_status_t::ACAMERA_OK {
        Ok(())
    } else {
        bail!("{what} failed: {status:?}")
    }
}

fn find_host_visible_memory_type_index(
    device: &Device,
    requirements: MemoryRequirements,
) -> Result<u32> {
    let memory_properties = device.physical_device().memory_properties();
    let required = MemoryPropertyFlags::HOST_VISIBLE | MemoryPropertyFlags::HOST_COHERENT;

    memory_properties
        .memory_types
        .iter()
        .enumerate()
        .position(|(idx, ty)| {
            (requirements.memory_type_bits & (1 << idx)) != 0 && ty.property_flags.contains(required)
        })
        .map(|idx| idx as u32)
        .context("no host-visible, host-coherent memory type found for camera staging image")
}
