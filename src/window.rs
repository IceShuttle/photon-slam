use crate::{
    fps::FpsCounter,
    trace_return,
    utils::camera::linux::{CameraCapture, CameraConfig},
    vulkan::{
        context::VulkanContext,
        disp::RenderContext,
        shaders::{
            fast::{self, FastPass},
            orb::OrbPass,
            yuvy_to_r8::YuvyToR8Pass,
        },
    },
};
use anyhow::Result;
use std::{sync::Arc, time::SystemTime};

use vulkano::{
    command_buffer::{
        AutoCommandBufferBuilder, BlitImageInfo, CommandBufferUsage, PrimaryAutoCommandBuffer,
    },
    image::Image,
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

macro_rules! compute_groups2D {
    ($extent:expr,$grp_size:expr) => {
        [
            $extent[0].div_ceil($grp_size),
            $extent[1].div_ceil($grp_size),
            1,
        ]
    };
}

/// Top-level application state.
///
/// Pipeline: camera → YUVY→R8 → FAST-9 corners → ORB orientation → display.
pub struct App<'a> {
    vk_context: VulkanContext,
    yuvy_pass: YuvyToR8Pass,
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

        // Camera first — need one frame to size the YUVY output image
        // (the camera image extent at width/2 is used to derive the
        // full-resolution output extent).
        let mut cam = CameraCapture::new(vk_context.device.clone(), &CameraConfig::default())?;
        let (cam_idx, cam_img) = cam.capture()?;

        // YUVY→R8: reads YUYV from the camera image, writes full-res
        // luminance.  The camera image is only used for extent here;
        // per-frame images come from capture/release in window_event.
        let yuvy_pass = YuvyToR8Pass::new(&vk_context, cam_img)?;
        cam.release(cam_idx)?;
        tracing::info!("YUVY→R8 pass created");

        // FAST-9: luminance → corner score mask.
        let fast_pass = FastPass::new(&vk_context, yuvy_pass.output_image.clone())?;
        tracing::info!("FAST-9 pass created");

        // ORB: luminance + FAST mask → orientation hue.
        let orb_pass = OrbPass::new(
            &vk_context,
            yuvy_pass.output_image.clone(),
            fast_pass.output_image.clone(),
        )?;
        tracing::info!("ORB pass created");

        Ok(Self {
            vk_context,
            yuvy_pass,
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

    fn extract_grayscale_from_cam_img(&self, cam_img: Arc<Image>) -> Result<()> {
        let mut extract_cmd_builder = self.auto_cmd_builder()?;

        // 1. YUVY→R8: extract luminance from the camera frame.
        let lum_extent = self.yuvy_pass.output_image.extent();
        let lum_groups = [lum_extent[0].div_ceil(16), lum_extent[1].div_ceil(16), 1];
        self.yuvy_pass
            .dispatch(&mut extract_cmd_builder, cam_img, lum_groups)?;

        let extract_cmd = extract_cmd_builder.build()?;

        let _ = vulkano::sync::now(self.vk_context.device.clone())
            .then_execute(self.vk_context.queue.clone(), extract_cmd)?
            .then_signal_fence_and_flush()?;
        Ok(())
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

                // Capture the latest camera frame.
                let (cam_idx, cam_img) = match self.cam.capture() {
                    Ok(val) => val,
                    Err(e) => {
                        trace_return!(e);
                    }
                };

                if let Err(e) = self.extract_grayscale_from_cam_img(cam_img) {
                    self.cam.release(cam_idx).unwrap();
                    trace_return!(e);
                }
                self.cam.release(cam_idx).unwrap();

                let mut process_cmd_builder = self.auto_cmd_builder().unwrap();

                // 2. FAST-9: corner detection on luminance.
                let fast_extent = self.fast_pass.output_image.extent();
                let fast_groups = compute_groups2D!(fast_extent, 16);
                self.fast_pass
                    .dispatch(
                        &mut process_cmd_builder,
                        fast::FastInputs { threshold: 0.02 },
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
