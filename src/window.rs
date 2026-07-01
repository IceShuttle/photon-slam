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
    window: Option<Arc<Window>>,
    surface: Option<Arc<Surface>>,
    device: Arc<Device>,
    queue: Arc<Queue>,
    image: Arc<Image>,
    swapchain: Option<Arc<Swapchain>>,
    swapchain_images: Option<Vec<Arc<Image>>>,
    cmd_buffer_allocator: Arc<dyn CommandBufferAllocator>,
}
impl App {
    pub fn new(
        instance: Arc<Instance>,
        physical_device: Arc<PhysicalDevice>,
        device: Arc<Device>,
        queue: Arc<Queue>,
        image: Arc<Image>,
        swapchain: Option<Arc<Swapchain>>,
        swapchain_images: Option<Vec<Arc<Image>>>,
        cmd_buffer_allocator: Arc<dyn CommandBufferAllocator>,
    ) -> Self {
        Self {
            instance,
            window: None,
            surface: None,
            physical_device,
            device,
            queue,
            image,
            swapchain,
            swapchain_images,
            cmd_buffer_allocator,
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
        self.swapchain = Some(swapchain);
        self.swapchain_images = Some(swapchain_images);

        // let render_pass = vulkano::single_pass_renderpass!(self.device.clone(),);

        // println!("{:?}", swapchain);
        // println!("Swapchain Size {}", swapchain_images.len());

        self.window = Some(window);
        self.surface = Some(surface);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                println!("The close button was pressed; stopping");
                event_loop.exit();
            }
            WindowEvent::RedrawRequested => {
                // match self.window.as_ref().unwrap().is_visible() {
                //     Some(val) => println!("{val}!"),
                //     None => println!("Wayland Magic!"),
                // }
                let result =
                    swapchain::acquire_next_image(self.swapchain.as_ref().unwrap().clone(), None);

                let (img_idx, _, acquire_future) = match result {
                    Ok(val) => val,
                    Err(_) => {
                        self.window.as_ref().unwrap().request_redraw();
                        return;
                    }
                };
                let mut present_builder = AutoCommandBufferBuilder::primary(
                    self.cmd_buffer_allocator.clone(),
                    self.queue.queue_family_index(),
                    CommandBufferUsage::OneTimeSubmit,
                )
                .unwrap();

                // let img = self.swapchain_images.unwrap();

                present_builder
                    .blit_image(BlitImageInfo::images(
                        self.image.clone(),
                        self.swapchain_images.as_ref().unwrap()[img_idx as usize].clone(),
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
                    SwapchainPresentInfo::swapchain_image_index(
                        self.swapchain.as_ref().unwrap().clone(),
                        img_idx,
                    ),
                )
                .then_signal_fence_and_flush();

                self.window.as_ref().unwrap().request_redraw();
            }
            _ => (),
        }
    }
}
