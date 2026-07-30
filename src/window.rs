use crate::{
    fps::FpsCounter,
    utils::camera::{CameraCapture, CameraConfig},
    vulkan::{context::VulkanContext, disp::RenderContext, shaders::yuvy_to_r8::YuvyToR8Pass},
};
use anyhow::Result;
use std::time::SystemTime;

use vulkano::{
    command_buffer::{
        AutoCommandBufferBuilder, BlitImageInfo, CommandBufferUsage, CopyImageToBufferInfo,
    },
    image::ImageLayout,
    swapchain::{self, SwapchainPresentInfo},
    sync::GpuFuture,
};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    window::WindowId,
};

/// Top-level application state.
///
/// Owns the Vulkan context, the YUVY→R8 compute pass, the camera, and the
/// on-screen display.
pub struct App<'a> {
    vk_context: VulkanContext,
    pass: YuvyToR8Pass,
    rcx: Option<RenderContext>,
    fps: FpsCounter,
    _start_time: SystemTime,
    cam: CameraCapture<'a>,
}

impl App<'_> {
    /// Create the app: initialise Vulkan, create the camera, build the
    /// YUVY→R8 compute pipeline.
    pub fn new(event_loop: &EventLoop<()>) -> Result<Self> {
        let vk_context = VulkanContext::new(Some(event_loop))?;

        // Camera first — we need its dimensions for the pass.
        let cam = CameraCapture::new(&vk_context, &CameraConfig::default())?;

        let pass = YuvyToR8Pass::new(&vk_context, cam.width, cam.height)?;
        tracing::info!("YUVY→R8 pass created ({}×{})", cam.width, cam.height);

        Ok(Self {
            vk_context,
            pass,
            rcx: None,
            fps: FpsCounter::new(),
            _start_time: SystemTime::now(),
            cam,
        })
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

                let (cam_idx, cam_img) = self.cam.capture().unwrap();

                let mut cmd = AutoCommandBufferBuilder::primary(
                    self.vk_context.cmd_buffer_allocator.clone(),
                    self.vk_context.queue.queue_family_index(),
                    CommandBufferUsage::OneTimeSubmit,
                )
                .unwrap();

                // Copy raw YUYV bytes from the camera's linear DMABUF
                // image into the pass's storage buffer.
                let mut copy_info =
                    CopyImageToBufferInfo::image_buffer(cam_img, self.pass.input_buffer.clone());
                copy_info.src_image_layout = ImageLayout::General;
                cmd.copy_image_to_buffer(copy_info).unwrap();

                // V4L2 buffer is now safe to release — the raw bytes are
                // in the storage buffer.
                self.cam.release(cam_idx).unwrap();

                // Dispatch the YUVY→R8 luminance-extraction shader.
                let extent = self.pass.output_image.extent();
                let group_counts = [extent[0].div_ceil(16), extent[1].div_ceil(16), 1];
                self.pass.dispatch(&mut cmd, group_counts).unwrap();

                let (img_idx, _, acquire_future) = match result {
                    Ok(val) => val,
                    Err(_) => {
                        rcx.window.request_redraw();
                        return;
                    }
                };

                // Blit the luminance output onto the swapchain image.
                cmd.blit_image(BlitImageInfo::images(
                    self.pass.output_image.clone(),
                    rcx.swapchain_images[img_idx as usize].clone(),
                ))
                .unwrap();

                let cmd = match cmd.build() {
                    Ok(val) => val,
                    Err(_) => return,
                };

                let future = match acquire_future.then_execute(self.vk_context.queue.clone(), cmd) {
                    Ok(val) => val,
                    Err(_) => return,
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
