//! Camera2 NDK capture → Vulkan image, zero-copy via
//! `VK_ANDROID_external_memory_android_hardware_buffer`.
//!
//! Pipeline:
//! 1. `ACameraDevice` streams into an `AImageReader` configured with
//!    `GPU_SAMPLED_IMAGE` usage, so each delivered `AImage` is backed by a
//!    GPU-importable `AHardwareBuffer`.
//! 2. On this hardware the buffer's pixel layout is opaque/vendor-defined
//!    (`vkGetAndroidHardwareBufferPropertiesANDROID` reports
//!    `format = UNDEFINED` with a nonzero `externalFormat`, and no
//!    `STORAGE_IMAGE` format feature) — it can only ever be read through a
//!    combined image sampler with an immutable `VkSamplerYcbcrConversion`,
//!    never as a storage image. Vulkano's safe image/memory APIs can't
//!    express `VkExternalFormatANDROID`, so the `VkImage`/`VkDeviceMemory`
//!    are built with raw Vulkan calls and wrapped via vulkano's
//!    `from_handle` constructors.
//! 3. `AImageReader`'s buffer pool is small and fixed (`BUFFER_COUNT`
//!    distinct gralloc buffers cycling forever), so imports are cached by
//!    `AHardwareBuffer` id: each buffer is imported once, then every later
//!    capture of it is a cache hit — no per-frame Vulkan object creation.
//! 4. The conversion uses the YCbCr-identity model, which skips the
//!    YCbCr→RGB matrix but still fixes luma/chroma to specific channels
//!    per the Vulkan multi-planar format spec (G=Y, B=Cb, R=Cr) — so a
//!    sample's `.g` component is raw luminance, not `.r`.
//!    `YuvyToR8Pass`'s android shader variant samples `.g` directly.
//! 5. `release()` is a no-op: the source `AImage` (and the driver-owned
//!    gralloc slot behind it) is freed at the end of `capture()` once its
//!    `AHardwareBuffer` is imported or already cached; Vulkan itself
//!    acquires an independent reference to the buffer during import, so
//!    the memory stays valid after the `AImage` is gone.

use super::CameraConfig;
use crate::vulkan::{context::VulkanContext, shaders::yuvy_to_r8::YuvyToR8Pass};
use anyhow::{bail, Context, Result};
use ash::vk;
use ndk::{
    hardware_buffer::{HardwareBuffer, HardwareBufferUsage},
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
    command_buffer::{AutoCommandBufferBuilder, CommandBufferUsage},
    device::Device,
    format::Format,
    image::{
        sampler::ycbcr::{
            SamplerYcbcrConversion, SamplerYcbcrConversionCreateInfo, SamplerYcbcrModelConversion,
            SamplerYcbcrRange,
        },
        sys::RawImage,
        Image, ImageCreateInfo, ImageLayout, ImageTiling, ImageType, ImageUsage,
    },
    memory::{DedicatedAllocation, DeviceMemory, ExternalMemoryHandleTypes, MemoryAllocateInfo},
    sync::GpuFuture,
    VulkanObject,
};

// Link the Camera2 NDK system library (`libcamera2ndk.so`, API 24+).
// `ndk-sys` declares the `ACamera*` symbols but never emits the link
// directive, so without this the final APK link fails with undefined
// symbols.
#[link(name = "camera2ndk")]
unsafe extern "C" {}

/// Number of `AImageReader` buffers Camera2 cycles through; also the
/// eventual size of the zero-copy import cache.
const BUFFER_COUNT: usize = 4;

/// A gralloc buffer imported once and reused for as long as `AImageReader`
/// keeps cycling it back.
struct CachedFrame {
    ahb_id: u64,
    image: Arc<Image>,
    // The image doesn't own or track this (`ImageMemory::External`), so it
    // must be kept alive here for as long as `image` is in use.
    _memory: DeviceMemory,
}

/// Camera2 NDK capture, producing zero-copy GPU-imported camera frames.
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
    vk_context: VulkanContext,
    cache: Vec<CachedFrame>,
    /// `VkSamplerYcbcrConversion` for this camera's (stable, opaque) pixel
    /// format, created from the first captured frame. `None` until then.
    ycbcr_conversion: Option<Arc<SamplerYcbcrConversion>>,
    /// YUYV(opaque)→R8 conversion pass, lazily built by `captureY()` on
    /// the first captured frame (needs `ycbcr_conversion` + a real image
    /// to size the output).
    yuvy_pass: Option<YuvyToR8Pass>,
    config: CameraConfig,
    _marker: PhantomData<&'a ()>,
}

impl AndroidCam<'_> {
    /// Opens the first available camera and starts a `YUV_420_888`
    /// preview stream at the requested resolution, backed by
    /// GPU-importable `AHardwareBuffer`s.
    pub fn new(vk_context: &VulkanContext, config: &CameraConfig) -> Result<Self> {
        let width = config.width;
        let height = config.height;
        let hz = config.hz.max(1);

        // 1. Image reader + listener: signals `capture()` whenever the
        //    camera has delivered a new frame. GPU_SAMPLED_IMAGE makes
        //    each AImage's buffer importable into Vulkan.
        let mut image_reader = ImageReader::new_with_usage(
            width as i32,
            height as i32,
            ImageFormat::YUV_420_888,
            HardwareBufferUsage::GPU_SAMPLED_IMAGE,
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
        check(open_status, "ACameraManager_openCamera")
            .inspect_err(|_| unsafe { ffi::ACameraManager_delete(manager) })?;

        // From here on, tear down manager+device on any error via a guard.
        let result = Self::finish_setup(
            vk_context,
            config,
            image_reader,
            frame_rx,
            window,
            manager,
            device,
            hz,
        );
        if result.is_err() {
            unsafe {
                ffi::ACameraDevice_close(device);
                ffi::ACameraManager_delete(manager);
            }
        }
        result
    }

    /// Builds the capture request/session. Split out of `new()` so the
    /// open camera manager/device can be torn down uniformly on any
    /// failure in this half of setup.
    fn finish_setup(
        vk_context: &VulkanContext,
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
            unsafe {
                ffi::ACaptureSessionOutput_create(window.ptr().as_ptr(), &mut session_output)
            },
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

        tracing::info!(
            "Camera2: {}x{} YUV_420_888 @ {}hz (zero-copy)",
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
            vk_context: vk_context.clone(),
            cache: Vec::with_capacity(BUFFER_COUNT),
            ycbcr_conversion: None,
            yuvy_pass: None,
            config: CameraConfig {
                pixel_format: Format::B8G8R8G8_422_UNORM,
                width: config.width,
                height: config.height,
                hz,
            },
            _marker: PhantomData,
        })
    }

    /// Waits for the next camera frame. The first time a given gralloc
    /// buffer is seen, its `AHardwareBuffer` is imported zero-copy into a
    /// Vulkan image; every later capture of the same (cycled) buffer
    /// reuses the cached image.
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

        let ahb = image
            .hardware_buffer()
            .context("AImage_getHardwareBuffer failed")?;
        let ahb_id = ahb.id().context("AHardwareBuffer_getId failed")?;

        if let Some(index) = self.cache.iter().position(|c| c.ahb_id == ahb_id) {
            // `image`/`ahb` drop here, returning the gralloc buffer to the
            // camera. Vulkan already holds its own reference to the
            // underlying memory (acquired during import), so the cached
            // Vulkan image stays valid regardless.
            return Ok((index, self.cache[index].image.clone()));
        }

        let (vk_image, memory, format_props) =
            import_hardware_buffer(&self.vk_context.device, &ahb)?;
        if self.ycbcr_conversion.is_none() {
            self.ycbcr_conversion = Some(create_ycbcr_conversion(
                &self.vk_context.device,
                &format_props,
            )?);
        }

        let index = self.cache.len();
        self.cache.push(CachedFrame {
            ahb_id,
            image: vk_image.clone(),
            _memory: memory,
        });
        tracing::debug!("Camera buffer {ahb_id} imported zero-copy (slot {index})");

        Ok((index, vk_image))
    }

    /// No-op: buffers are imported once and cached forever, so there is no
    /// per-frame handle to hand back.
    pub fn release(&mut self, _index: usize) -> Result<()> {
        Ok(())
    }

    /// Captures the next frame, converts its opaque YUV layout to
    /// luminance on the GPU, and releases the buffer back to the camera
    /// driver (a no-op on Android — see [`release`](Self::release)).
    /// Returns the persistent R8_UNORM output image — the same `Arc`
    /// every call, its contents overwritten each frame.
    #[allow(non_snake_case)]
    pub fn captureY(&mut self) -> Result<Arc<Image>> {
        let (index, cam_img) = self.capture()?;

        if self.yuvy_pass.is_none() {
            self.yuvy_pass = Some(YuvyToR8Pass::new(
                &self.vk_context.device,
                &self.vk_context.memory_allocator,
                cam_img.clone(),
                self.ycbcr_conversion.clone(),
            )?);
        }
        let yuvy_pass = self.yuvy_pass.as_ref().unwrap();

        let mut cmd_builder = AutoCommandBufferBuilder::primary(
            self.vk_context.cmd_buffer_allocator.clone(),
            self.vk_context.queue.queue_family_index(),
            CommandBufferUsage::OneTimeSubmit,
        )?;
        let extent = yuvy_pass.output_image.extent();
        let groups = compute_groups2D!(extent, 16);
        yuvy_pass.dispatch(&mut cmd_builder, cam_img, groups)?;
        let output_image = yuvy_pass.output_image.clone();
        let cmd = cmd_builder.build()?;

        vulkano::sync::now(self.vk_context.device.clone())
            .then_execute(self.vk_context.queue.clone(), cmd)?
            .then_signal_fence_and_flush()?
            .wait(None)?;

        self.release(index)?;
        Ok(output_image)
    }

    /// Returns the Camera config
    pub fn config(&self) -> CameraConfig {
        self.config
    }
}

impl Drop for AndroidCam<'_> {
    fn drop(&mut self) {
        // Each cached image was wrapped via `from_handle_borrowed` (to
        // avoid a vulkano-internal memory-requirements query that panics
        // on this hardware's opaque AHardwareBuffer-backed images), so it
        // is not destroyed automatically — do it here, before `self.cache`
        // drops its `DeviceMemory` entries (Vulkan requires destroying a
        // bound resource before freeing its memory).
        for frame in &self.cache {
            unsafe {
                (self.vk_context.device.fns().v1_0.destroy_image)(
                    self.vk_context.device.handle(),
                    frame.image.handle(),
                    ptr::null(),
                );
            }
        }

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

unsafe extern "C" fn on_device_disconnected(
    _context: *mut c_void,
    _device: *mut ffi::ACameraDevice,
) {
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

fn vk_check(result: vk::Result, what: &str) -> Result<()> {
    if result == vk::Result::SUCCESS {
        Ok(())
    } else {
        bail!("{what} failed: {result:?}")
    }
}

/// Imports `ahb`'s memory directly as a sampled Vulkan image — no CPU
/// copy. Its pixel layout is opaque (vendor-defined), so vulkano's safe
/// image/memory APIs can't express it: the `VkImage`/`VkDeviceMemory` are
/// built with raw Vulkan calls and wrapped via vulkano's `from_handle`
/// constructors.
fn import_hardware_buffer(
    device: &Arc<Device>,
    ahb: &HardwareBuffer,
) -> Result<(
    Arc<Image>,
    DeviceMemory,
    vk::AndroidHardwareBufferFormatPropertiesANDROID<'static>,
)> {
    let ahb_ptr: *mut vk::AHardwareBuffer = ahb.as_ptr().cast();
    let desc = ahb.describe();

    let mut format_props = vk::AndroidHardwareBufferFormatPropertiesANDROID::default();
    let (allocation_size, memory_type_bits) = {
        let mut props =
            vk::AndroidHardwareBufferPropertiesANDROID::default().push_next(&mut format_props);
        vk_check(
            unsafe {
                (device
                    .fns()
                    .android_external_memory_android_hardware_buffer
                    .get_android_hardware_buffer_properties_android)(
                    device.handle(),
                    ahb_ptr.cast_const(),
                    &mut props,
                )
            },
            "vkGetAndroidHardwareBufferPropertiesANDROID",
        )?;
        (props.allocation_size, props.memory_type_bits)
    };

    // vkCreateImage: `format` is UNDEFINED — the real (opaque) pixel
    // layout is carried by `externalFormat` instead, which only a
    // combined image sampler with a matching `VkSamplerYcbcrConversion`
    // can read (confirmed by `format_props.format_features` reporting
    // SAMPLED_IMAGE but no STORAGE_IMAGE support on this hardware).
    let mut external_format_info =
        vk::ExternalFormatANDROID::default().external_format(format_props.external_format);
    let mut external_memory_info = vk::ExternalMemoryImageCreateInfo::default()
        .handle_types(vk::ExternalMemoryHandleTypeFlags::ANDROID_HARDWARE_BUFFER_ANDROID);
    let image_create_info_vk = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .format(vk::Format::UNDEFINED)
        .extent(vk::Extent3D {
            width: desc.width,
            height: desc.height,
            depth: 1,
        })
        .mip_levels(1)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::OPTIMAL)
        .usage(vk::ImageUsageFlags::SAMPLED)
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .push_next(&mut external_memory_info)
        .push_next(&mut external_format_info);

    let mut image_handle = vk::Image::null();
    vk_check(
        unsafe {
            (device.fns().v1_0.create_image)(
                device.handle(),
                &image_create_info_vk,
                ptr::null(),
                &mut image_handle,
            )
        },
        "vkCreateImage",
    )?;

    let vk_create_info = ImageCreateInfo {
        image_type: ImageType::Dim2d,
        // The real `VkImage` was created with `format = UNDEFINED` (its
        // actual layout is opaque, carried by `externalFormat`). Vulkano's
        // own bookkeeping needs a format with a `Color` aspect to compute
        // a non-empty subresource range (`Format::UNDEFINED` has none,
        // which panics on an empty-range assertion) — a placeholder
        // concrete format is fine here since nothing reads it back for
        // GPU interpretation (the image view below hardcodes `UNDEFINED`
        // explicitly, matching the real object).
        format: Format::R8G8B8A8_UNORM,
        extent: [desc.width, desc.height, 1],
        usage: ImageUsage::SAMPLED,
        tiling: ImageTiling::Optimal,
        initial_layout: ImageLayout::Undefined,
        external_memory_handle_types: ExternalMemoryHandleTypes::ANDROID_HARDWARE_BUFFER,
        ..Default::default()
    };
    // SAFETY: `image_handle` was just created from `device` with a
    // matching `ImageCreateInfo`, has no memory bound yet, and is
    // destroyed manually in `AndroidCam::drop` (see there for why
    // `from_handle`'s unconditional memory-requirements query can't be
    // used here).
    let raw_image =
        unsafe { RawImage::from_handle_borrowed(device.clone(), image_handle, vk_create_info) }
            .context("failed to wrap imported VkImage")?;

    // vkAllocateMemory: import the AHardwareBuffer's memory, dedicated to
    // the image just created.
    let memory_type_index = (0..32)
        .find(|i| (memory_type_bits & (1 << i)) != 0)
        .context("no compatible memory type for AHardwareBuffer import")?;
    let mut dedicated_alloc_info = vk::MemoryDedicatedAllocateInfo::default().image(image_handle);
    let mut import_info = vk::ImportAndroidHardwareBufferInfoANDROID::default().buffer(ahb_ptr);
    let alloc_info_vk = vk::MemoryAllocateInfo::default()
        .allocation_size(allocation_size)
        .memory_type_index(memory_type_index)
        .push_next(&mut dedicated_alloc_info)
        .push_next(&mut import_info);

    let mut memory_handle = vk::DeviceMemory::null();
    vk_check(
        unsafe {
            (device.fns().v1_0.allocate_memory)(
                device.handle(),
                &alloc_info_vk,
                ptr::null(),
                &mut memory_handle,
            )
        },
        "vkAllocateMemory (AHardwareBuffer import)",
    )?;
    vk_check(
        unsafe {
            (device.fns().v1_0.bind_image_memory)(device.handle(), image_handle, memory_handle, 0)
        },
        "vkBindImageMemory",
    )?;

    // SAFETY: `memory_handle` was just allocated from `device` with the
    // parameters mirrored below.
    let device_memory = unsafe {
        DeviceMemory::from_handle(
            device.clone(),
            memory_handle,
            MemoryAllocateInfo {
                allocation_size,
                memory_type_index,
                dedicated_allocation: Some(DedicatedAllocation::Image(&raw_image)),
                ..Default::default()
            },
        )
    };
    // SAFETY: memory matching `raw_image`'s requirements was just bound
    // to it above via `vkBindImageMemory`.
    let vk_image = Arc::new(unsafe { raw_image.assume_bound() });

    Ok((vk_image, device_memory, format_props))
}

/// Creates the `VkSamplerYcbcrConversion` needed to sample `externalFormat`
/// images, using the YCbCr-identity model so a sample's `.g` component is
/// raw luminance (per the Vulkan multi-planar format spec: G=Y, B=Cb,
/// R=Cr) with no color-space matrix applied.
fn create_ycbcr_conversion(
    device: &Arc<Device>,
    format_props: &vk::AndroidHardwareBufferFormatPropertiesANDROID,
) -> Result<Arc<SamplerYcbcrConversion>> {
    let mut external_format_info =
        vk::ExternalFormatANDROID::default().external_format(format_props.external_format);
    let create_info_vk = vk::SamplerYcbcrConversionCreateInfo::default()
        .format(vk::Format::UNDEFINED)
        .ycbcr_model(vk::SamplerYcbcrModelConversion::YCBCR_IDENTITY)
        .ycbcr_range(vk::SamplerYcbcrRange::ITU_FULL)
        .components(vk::ComponentMapping::default())
        .x_chroma_offset(vk::ChromaLocation::COSITED_EVEN)
        .y_chroma_offset(vk::ChromaLocation::COSITED_EVEN)
        .chroma_filter(vk::Filter::LINEAR)
        .force_explicit_reconstruction(false)
        .push_next(&mut external_format_info);

    let mut handle = vk::SamplerYcbcrConversion::null();
    vk_check(
        unsafe {
            (device
                .fns()
                .khr_sampler_ycbcr_conversion
                .create_sampler_ycbcr_conversion_khr)(
                device.handle(),
                &create_info_vk,
                ptr::null(),
                &mut handle,
            )
        },
        "vkCreateSamplerYcbcrConversionKHR",
    )?;

    // SAFETY: `handle` was just created from `device`; the create info
    // below mirrors what was passed to the raw call above (format left as
    // `UNDEFINED` — the real format is `format_props.external_format`,
    // which vulkano's `SamplerYcbcrConversionCreateInfo` cannot express).
    Ok(unsafe {
        SamplerYcbcrConversion::from_handle(
            device.clone(),
            handle,
            SamplerYcbcrConversionCreateInfo {
                format: Format::UNDEFINED,
                ycbcr_model: SamplerYcbcrModelConversion::YcbcrIdentity,
                ycbcr_range: SamplerYcbcrRange::ItuFull,
                chroma_filter: vulkano::image::sampler::Filter::Linear,
                ..Default::default()
            },
        )
    })
}
