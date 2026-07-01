use std::sync::Arc;
use vulkano::command_buffer::allocator::CommandBufferAllocator;
use vulkano::command_buffer::{AutoCommandBufferBuilder, BlitImageInfo, CommandBufferUsage};
use vulkano::device::physical::PhysicalDevice;
use vulkano::device::{Device, Queue};
use vulkano::format::Format;
use vulkano::image::{Image, ImageUsage};
use vulkano::instance::Instance;
use vulkano::swapchain::{
    self, Surface, SurfaceInfo, Swapchain, SwapchainCreateInfo, SwapchainPresentInfo,
};
use vulkano::sync::GpuFuture;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowId};

pub struct App {
    instance: Arc<Instance>,
    physical_device: Arc<PhysicalDevice>,
    device: Arc<Device>,
    queue: Arc<Queue>,
    image: Arc<Image>,
    cmd_buffer_allocator: Arc<dyn CommandBufferAllocator>,
    rcx: Option<RenderContext>,
}

pub struct RenderContext {
    window: Arc<Window>,
    surface: Arc<Surface>,
    swapchain: Arc<Swapchain>,
    swapchain_images: Vec<Arc<Image>>,
}

impl App {
    pub fn new(
        instance: Arc<Instance>,
        physical_device: Arc<PhysicalDevice>,
        device: Arc<Device>,
        queue: Arc<Queue>,
        image: Arc<Image>,
        cmd_buffer_allocator: Arc<dyn CommandBufferAllocator>,
    ) -> Self {
        Self {
            instance,
            physical_device,
            device,
            queue,
            image,
            cmd_buffer_allocator,
            rcx: None,
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = Arc::new(
            event_loop
                .create_window(Window::default_attributes())
                .unwrap(),
        );
        let surface = Surface::from_window(self.instance.clone(), window.clone()).unwrap();

        let caps = self
            .physical_device
            .surface_capabilities(&surface, Default::default())
            .unwrap();

        let dimensions = window.inner_size();
        let composite_alpha = caps.supported_composite_alpha.into_iter().next().unwrap();

        let formats = self
            .physical_device
            .surface_formats(&surface, SurfaceInfo::default())
            .unwrap();

        let image_format = match formats.iter().find(|(fmt, _)| {
            matches!(
                *fmt,
                Format::B8G8R8A8_SRGB
                    | Format::R8G8B8A8_SRGB
                    | Format::B8G8R8A8_UNORM
                    | Format::R8G8B8A8_UNORM
            )
        }) {
            Some(f) => f.0,
            None => {
                println!("Using format {:?}", &formats[0].0);
                formats[0].0
            }
        };

        let (swapchain, swapchain_images) = Swapchain::new(
            self.device.clone(),
            surface.clone(),
            SwapchainCreateInfo {
                min_image_count: caps.min_image_count + 1, // How many buffers to use in the swapchain
                image_format,
                image_extent: dimensions.into(),
                image_usage: ImageUsage::COLOR_ATTACHMENT | ImageUsage::TRANSFER_DST, // What the images are going to be used for
                composite_alpha,
                ..Default::default()
            },
        )
        .unwrap();
        println!("Swapchain Initialized!");
        self.rcx = Some(RenderContext {
            swapchain,
            swapchain_images,
            window,
            surface,
        });
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                println!("The close button was pressed; stopping");
                event_loop.exit();
            }
            WindowEvent::RedrawRequested => {
                let rcx = self.rcx.as_ref().unwrap();

                let result = swapchain::acquire_next_image(rcx.swapchain.clone(), None);

                let (img_idx, _, acquire_future) = match result {
                    Ok(val) => val,
                    Err(_) => {
                        rcx.window.request_redraw();
                        return;
                    }
                };
                let mut present_builder = AutoCommandBufferBuilder::primary(
                    self.cmd_buffer_allocator.clone(),
                    self.queue.queue_family_index(),
                    CommandBufferUsage::OneTimeSubmit,
                )
                .unwrap();

                present_builder
                    .blit_image(BlitImageInfo::images(
                        self.image.clone(),
                        rcx.swapchain_images[img_idx as usize].clone(),
                    ))
                    .unwrap();

                let cmd = match present_builder.build() {
                    Ok(val) => val,
                    Err(_) => return,
                };

                let future = match acquire_future.then_execute(self.queue.clone(), cmd) {
                    Ok(val) => val,
                    Err(_) => return,
                };

                let _ = swapchain::present(
                    future,
                    self.queue.clone(),
                    SwapchainPresentInfo::swapchain_image_index(rcx.swapchain.clone(), img_idx),
                )
                .then_signal_fence_and_flush();

                rcx.window.request_redraw();
            }
            _ => (),
        }
    }
}
