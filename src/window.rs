use crate::{
    compute_groups2D,
    fps::FpsCounter,
    trace_return,
    utils::camera::CameraConfig,
    vulkan::{
        context::VulkanContext,
        disp::RenderContext,
        shaders::{
            fast::{self, FastPass},
            orb::OrbPass,
        },
    },
};
use anyhow::Result;
use std::time::SystemTime;

use vulkano::{
    command_buffer::{
        AutoCommandBufferBuilder, BlitImageInfo, CommandBufferUsage, PrimaryAutoCommandBuffer,
    },
    swapchain::{self, SwapchainPresentInfo},
    sync::GpuFuture,
    Validated, VulkanError,
};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    window::WindowId,
};

#[cfg(target_os = "linux")]
use crate::utils::camera::linux::V4lCapture;

#[cfg(target_os = "android")]
use crate::utils::camera::android::AndroidCam;
#[cfg(target_os = "android")]
type CameraCapture<'a> = AndroidCam<'a>;

#[cfg(target_os = "linux")]
type CameraCapture<'a> = V4lCapture<'a>;

/// Top-level application state.
///
/// Pipeline: camera → YUVY→R8 → FAST-9 corners → ORB orientation → display.
pub struct App<'a> {
    vk_context: VulkanContext,
    fast_pass: FastPass,
    orb_pass: OrbPass,
    rcx: Option<RenderContext>,
    fps: FpsCounter,
    _start_time: SystemTime,
    cam: CameraCapture<'a>,
}

// struct ShaderPasses {}

impl App<'_> {
    /// Initialise Vulkan, create the camera, build all compute passes.
    pub fn new(event_loop: &EventLoop<()>) -> Result<Self> {
        let vk_context = VulkanContext::new(Some(event_loop))?;

        // Camera first: one warm-up `captureY()` (capture → YUVY→R8 →
        // release, all done by the camera) materializes the persistent
        // luminance image needed to size FAST/ORB.
        let mut cam = CameraCapture::new(&vk_context, &CameraConfig::default())?;
        let lum_image = cam.captureY()?;
        tracing::info!("Camera + YUVY→R8 pass created");

        // FAST-9: luminance → corner score mask.
        let fast_pass = FastPass::new(&vk_context, lum_image.clone())?;
        tracing::info!("FAST-9 pass created");

        // ORB: luminance + FAST mask → orientation hue.
        let orb_pass = OrbPass::new(&vk_context, lum_image, fast_pass.output_image.clone())?;
        tracing::info!("ORB pass created");

        Ok(Self {
            vk_context,
            fast_pass,
            orb_pass,
            rcx: None, // To be initialized on resume
            fps: FpsCounter::new(),
            _start_time: SystemTime::now(),
            cam,
        })
    }

    fn auto_cmd_builder(
        &self,
    ) -> Result<AutoCommandBufferBuilder<PrimaryAutoCommandBuffer>, Validated<VulkanError>> {
        AutoCommandBufferBuilder::primary(
            self.vk_context.cmd_buffer_allocator.clone(),
            self.vk_context.queue.queue_family_index(),
            CommandBufferUsage::OneTimeSubmit,
        )
    }
}

impl ApplicationHandler for App<'_> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let rcx = RenderContext::new(
            event_loop,
            self.vk_context.instance.clone(),
            self.vk_context.physical_device.clone(),
            self.vk_context.device.clone(),
        )
        .expect("Vulkan Initialization Failed");
        self.rcx = Some(rcx);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                tracing::info!("The close button was pressed; stopping");
                event_loop.exit();
            }
            WindowEvent::RedrawRequested => {
                let rcx = self.rcx.as_ref().unwrap();
                let result = swapchain::acquire_next_image(rcx.swapchain.clone(), None);

                // Capture the latest camera frame, convert to luminance,
                // and release the buffer — all handled by the camera.
                if let Err(e) = self.cam.captureY() {
                    trace_return!(e);
                }

                let mut process_cmd_builder = self.auto_cmd_builder().unwrap();

                // 2. FAST-9: corner detection on luminance.
                let fast_extent = self.fast_pass.output_image.extent();
                let fast_groups = compute_groups2D!(fast_extent, 16);
                self.fast_pass
                    .dispatch(
                        &mut process_cmd_builder,
                        fast::FastInputs { threshold: 0.01 },
                        fast_groups,
                    )
                    .unwrap();

                // 3. ORB: orientation at FAST corner locations.
                let orb_extent = self.orb_pass.output_image.extent();
                let orb_groups = compute_groups2D!(orb_extent, 16);
                self.orb_pass
                    .dispatch(&mut process_cmd_builder, orb_groups)
                    .unwrap();

                let (img_idx, _, acquire_future) = match result {
                    Ok(val) => val,
                    Err(_) => {
                        rcx.window.request_redraw();
                        return;
                    }
                };

                // 4. Blit ORB orientation output → swapchain.
                process_cmd_builder
                    .blit_image(BlitImageInfo::images(
                        self.orb_pass.output_image.clone(),
                        rcx.swapchain_images[img_idx as usize].clone(),
                    ))
                    .unwrap();

                let cmd = process_cmd_builder.build().unwrap();

                let future = match acquire_future.then_execute(self.vk_context.queue.clone(), cmd) {
                    Ok(val) => val,
                    Err(_) => {
                        return;
                    }
                };

                let _ = swapchain::present(
                    future,
                    self.vk_context.queue.clone(),
                    SwapchainPresentInfo::swapchain_image_index(rcx.swapchain.clone(), img_idx),
                )
                .then_signal_fence_and_flush();

                self.fps.tick();
                rcx.window.request_redraw();
            }
            _ => (),
        }
    }
}
