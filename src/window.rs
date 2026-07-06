use crate::{
    fps::FpsCounter,
    vulkan::{self, context::VulkanContext, disp::RenderContext, shaders::testing::TestingPass},
};
use anyhow::Result;
use std::time::SystemTime;

use vulkano::{
    command_buffer::{AutoCommandBufferBuilder, BlitImageInfo, CommandBufferUsage},
    swapchain::{self, SwapchainPresentInfo},
    sync::{self, GpuFuture},
};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    window::WindowId,
};

/// Top-level application state.
///
/// Owns the Vulkan context, the compute pass, and the on-screen display.
pub struct App {
    vk_context: VulkanContext,
    pass: TestingPass,
    rcx: Option<RenderContext>,
    fps: FpsCounter,
    start_time: SystemTime,
}

impl App {
    /// Create the app: initialise Vulkan, upload the test image, and build
    /// the compute pipeline.
    pub fn new(event_loop: &EventLoop<()>) -> Result<Self> {
        let ctx = VulkanContext::new(Some(event_loop))?;

        // Upload the compile-time-embedded test image to the GPU.
        let mut uploads = AutoCommandBufferBuilder::primary(
            ctx.cmd_buffer_allocator.clone(),
            ctx.queue.queue_family_index(),
            CommandBufferUsage::OneTimeSubmit,
        )?;
        let img = vulkan::image::load_image()?;
        let texture = vulkan::image::upload_image(&ctx.memory_allocator, &mut uploads, img)?;
        {
            let cmd_buff = uploads.build()?;
            sync::now(ctx.device.clone())
                .then_execute(ctx.queue.clone(), cmd_buff)?
                .flush()?;
            tracing::debug!("Image uploaded");
        }

        let pass = TestingPass::new(&ctx, texture)?;

        Ok(Self {
            vk_context: ctx,
            pass,
            rcx: None,
            fps: FpsCounter::new(),
            start_time: SystemTime::now(),
        })
    }
}

impl ApplicationHandler for App {
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

                let mut present_builder = AutoCommandBufferBuilder::primary(
                    self.vk_context.cmd_buffer_allocator.clone(),
                    self.vk_context.queue.queue_family_index(),
                    CommandBufferUsage::OneTimeSubmit,
                )
                .unwrap();

                // Dispatch the compute shader.
                let extent = self.pass.output_image.extent();
                let group_counts = [extent[0].div_ceil(16), extent[1].div_ceil(16), 1];
                let elapsed = SystemTime::now()
                    .duration_since(self.start_time)
                    .unwrap()
                    .as_millis() as u32;
                self.pass
                    .dispatch(&mut present_builder, elapsed, group_counts)
                    .unwrap();

                let (img_idx, _, acquire_future) = match result {
                    Ok(val) => val,
                    Err(_) => {
                        rcx.window.request_redraw();
                        return;
                    }
                };

                // Blit the compute output onto the swapchain image.
                present_builder
                    .blit_image(BlitImageInfo::images(
                        self.pass.output_image.clone(),
                        rcx.swapchain_images[img_idx as usize].clone(),
                    ))
                    .unwrap();

                let cmd = match present_builder.build() {
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
