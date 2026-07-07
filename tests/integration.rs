//! Human-in-the-loop shader integration test.
//!
//! Runs every compute pass on the static test image, displays the result in
//! a window, and waits for the developer to press:
//!
//! - `y` — pass this shader, advance to the next
//! - `n` — fail the test (panics)
//!
//! The test passes only when every shader has been accepted.

use std::sync::Arc;
use std::time::SystemTime;

use photon_slam::{
    utils::tracing::setup_tracing,
    vulkan::{
        self,
        context::VulkanContext,
        disp::RenderContext,
        shaders::{
            errors::ShaderDispatchError,
            fast::{FastInputs, FastPass},
            gaussian_blur::{GaussianBlurInputs, GaussianBlurPass},
            orb::OrbPass,
            testing::TestingPass,
        },
    },
};

use vulkano::{
    command_buffer::{
        AutoCommandBufferBuilder, BlitImageInfo, CommandBufferUsage, PrimaryAutoCommandBuffer,
    },
    image::Image,
    swapchain::{self, SwapchainPresentInfo},
    sync::{self, GpuFuture},
};
use winit::{
    application::ApplicationHandler,
    event::{ElementState, KeyEvent, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop},
    window::WindowId,
};

// Linux-only: allow EventLoop on a non-main thread (cargo test spawns tests
// in child threads).  The platform-specific `with_any_thread` flag disables
// the main-thread check.
#[cfg(target_os = "linux")]
use winit::platform::x11::EventLoopBuilderExtX11;

// ---------------------------------------------------------------------------
// Unified interface over all pass types
// ---------------------------------------------------------------------------

enum ShaderPass {
    Test(TestingPass),
    Fast(FastPass),
    GaussianBlur(GaussianBlurPass),
    Orb(OrbPass),
}

impl ShaderPass {
    fn output_image(&self) -> &Arc<Image> {
        match self {
            ShaderPass::Test(p) => &p.output_image,
            ShaderPass::Fast(p) => &p.output_image,
            ShaderPass::GaussianBlur(p) => &p.output_image,
            ShaderPass::Orb(p) => &p.output_image,
        }
    }

    fn dispatch(
        &self,
        cmd: &mut AutoCommandBufferBuilder<PrimaryAutoCommandBuffer>,
        groups: [u32; 3],
        time: u32,
    ) -> Result<(), ShaderDispatchError> {
        match self {
            ShaderPass::Test(p) => p.dispatch(cmd, time, groups),
            ShaderPass::Fast(p) => p.dispatch(cmd, FastInputs { threshold: 0.15 }, groups),
            ShaderPass::GaussianBlur(p) => p.dispatch(
                cmd,
                GaussianBlurInputs {
                    sigma: 1.5,
                    kernel_size: 11,
                },
                groups,
            ),
            ShaderPass::Orb(p) => p.dispatch(cmd, groups),
        }
    }

    fn label(&self) -> &'static str {
        match self {
            ShaderPass::Test(_) => "test (Gaussian blur + colour shift)",
            ShaderPass::Fast(_) => "FAST-9 corner detection",
            ShaderPass::GaussianBlur(_) => "Gaussian blur",
            ShaderPass::Orb(_) => "ORB intensity-centroid orientation",
        }
    }
}

// ---------------------------------------------------------------------------
// Application state
// ---------------------------------------------------------------------------

struct Tester {
    ctx: VulkanContext,
    passes: Vec<ShaderPass>,
    current: usize,
    rcx: Option<RenderContext>,
    start_time: SystemTime,
    fps: photon_slam::fps::FpsCounter,
}

impl Tester {
    fn new(event_loop: &EventLoop<()>) -> anyhow::Result<Self> {
        let ctx = VulkanContext::new(Some(event_loop))?;

        // Upload the test image once.  All passes share the same input.
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
        }

        // Build all passes upfront, chaining FAST output into ORB.
        let fast_pass = FastPass::new(&ctx, texture.clone())?;
        let orb_pass = OrbPass::new(&ctx, texture.clone(), fast_pass.output_image.clone())?;
        let gaussian_blur_pass = GaussianBlurPass::new(&ctx, texture.clone())?;
        let passes = vec![
            ShaderPass::Test(TestingPass::new(&ctx, texture)?),
            ShaderPass::GaussianBlur(gaussian_blur_pass),
            ShaderPass::Fast(fast_pass),
            ShaderPass::Orb(orb_pass),
        ];
        Ok(Self {
            ctx,
            passes,
            current: 0,
            rcx: None,
            start_time: SystemTime::now(),
            fps: photon_slam::fps::FpsCounter::new(),
        })
    }

    fn active(&self) -> &ShaderPass {
        &self.passes[self.current]
    }
}

impl ApplicationHandler for Tester {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let rcx = RenderContext::new(
            event_loop,
            self.ctx.instance.clone(),
            self.ctx.physical_device.clone(),
            self.ctx.device.clone(),
        )
        .ok();
        self.rcx = rcx;

        println!(
            "=== Shader test [{}/{}]: {} ===",
            self.current + 1,
            self.passes.len(),
            self.active().label()
        );
        println!("Click the window, then press  y  to pass this shader,  n  to fail.");
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }

            WindowEvent::Focused(true) => {
                println!("[window focused]");
                if let Some(rcx) = self.rcx.as_ref() {
                    rcx.window.request_redraw();
                }
            }
            WindowEvent::Focused(false) => {
                println!("[window unfocused — click the window to regain focus]");
            }

            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        logical_key: key,
                        text,
                        state: ElementState::Pressed,
                        ..
                    },
                ..
            } => {
                // Check typed text first (most reliable), then fall back to
                // logical_key::Character for platforms that don't populate `text`.
                let accept = || {
                    let from_text = text.as_deref().is_some_and(|t| t == "y" || t == "Y");
                    let from_key =
                        matches!(&key, winit::keyboard::Key::Character(c) if c == "y" || c == "Y");
                    from_text || from_key
                };
                let reject = || {
                    let from_text = text.as_deref().is_some_and(|t| t == "n" || t == "N");
                    let from_key =
                        matches!(&key, winit::keyboard::Key::Character(c) if c == "n" || c == "N");
                    from_text || from_key
                };

                if accept() {
                    println!("✓ Accepted: {}", self.active().label());
                    self.current += 1;
                    if self.current >= self.passes.len() {
                        println!("✅ All shaders passed!");
                        event_loop.exit();
                    } else {
                        println!(
                            "=== Shader test [{}/{}]: {} ===",
                            self.current + 1,
                            self.passes.len(),
                            self.active().label()
                        );
                        println!("Click the window, then press  y  to pass,  n  to fail.");
                        if let Some(rcx) = self.rcx.as_ref() {
                            rcx.window.request_redraw();
                        }
                    }
                    return;
                }

                if reject() {
                    eprintln!("❌ Rejected: {}", self.active().label());
                    panic!(
                        "Human-in-the-loop rejected shader '{}' — test failed",
                        self.active().label()
                    );
                }

                println!("[key: text={text:?}, logical={key:?}]");
            }

            WindowEvent::RedrawRequested => {
                // After the last shader is accepted and event_loop.exit()
                // is called, one final RedrawRequested may still fire before
                // the loop stops.  Guard against the out-of-bounds access.
                if self.current >= self.passes.len() {
                    return;
                }
                let rcx = match self.rcx.as_ref() {
                    Some(v) => v,
                    None => return,
                };

                self.fps.tick();

                let result = swapchain::acquire_next_image(rcx.swapchain.clone(), None);
                let mut cmd = AutoCommandBufferBuilder::primary(
                    self.ctx.cmd_buffer_allocator.clone(),
                    self.ctx.queue.queue_family_index(),
                    CommandBufferUsage::OneTimeSubmit,
                )
                .unwrap();

                // Dispatch the current shader pass.
                let extent = self.active().output_image().extent();
                let groups = [extent[0].div_ceil(16), extent[1].div_ceil(16), 1];
                let elapsed = SystemTime::now()
                    .duration_since(self.start_time)
                    .unwrap()
                    .as_millis() as u32;
                self.active().dispatch(&mut cmd, groups, elapsed).unwrap();

                let (img_idx, _, acquire_future) = match result {
                    Ok(v) => v,
                    Err(_) => {
                        rcx.window.request_redraw();
                        return;
                    }
                };

                // Blit the compute output onto the swapchain image.
                cmd.blit_image(BlitImageInfo::images(
                    self.active().output_image().clone(),
                    rcx.swapchain_images[img_idx as usize].clone(),
                ))
                .unwrap();

                let cmd = match cmd.build() {
                    Ok(v) => v,
                    Err(_) => return,
                };

                let future = match acquire_future.then_execute(self.ctx.queue.clone(), cmd) {
                    Ok(v) => v,
                    Err(_) => return,
                };

                let _ = swapchain::present(
                    future,
                    self.ctx.queue.clone(),
                    SwapchainPresentInfo::swapchain_image_index(rcx.swapchain.clone(), img_idx),
                )
                .then_signal_fence_and_flush();

                rcx.window.request_redraw();
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Test entry point
// ---------------------------------------------------------------------------

#[test]
#[ignore]
fn human_in_loop_shader_test() {
    setup_tracing();
    let event_loop = {
        #[cfg(target_os = "linux")]
        {
            let mut builder = EventLoop::builder();
            builder.with_any_thread(true);
            builder.build().unwrap()
        }
        #[cfg(not(target_os = "linux"))]
        EventLoop::new().unwrap()
    };

    let mut tester = Tester::new(&event_loop).unwrap();
    event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);
    event_loop.run_app(&mut tester).unwrap();
}
