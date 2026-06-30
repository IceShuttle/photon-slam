use std::sync::Arc;
use vulkano::device::physical::PhysicalDevice;
use vulkano::device::{Device, Queue};
use vulkano::image::view::ImageView;
use vulkano::image::{Image, ImageUsage};
use vulkano::instance::Instance;
use vulkano::swapchain::{self, Surface, SurfaceInfo, Swapchain, SwapchainCreateInfo};
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
    image: Arc<ImageView>,
}
impl App {
    pub fn new(
        instance: Arc<Instance>,
        physical_device: Arc<PhysicalDevice>,
        device: Arc<Device>,
        queue: Arc<Queue>,
        image: Arc<ImageView>,
    ) -> Self {
        Self {
            instance,
            window: None,
            surface: None,
            physical_device,
            device,
            queue,
            image,
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

        let image_format = self
            .physical_device
            .surface_formats(&surface, Default::default())
            .unwrap()[0]
            .0;

        let present_modes = self
            .physical_device
            .surface_present_modes(&surface, SurfaceInfo::default())
            .unwrap();

        let present_mode = if present_modes.contains(&swapchain::PresentMode::Mailbox) {
            println!("Using Mailbox PresentMode");
            swapchain::PresentMode::Mailbox
        } else if present_modes.contains(&swapchain::PresentMode::Immediate) {
            println!("Using Immediate PresentMode");
            swapchain::PresentMode::Immediate
        } else {
            println!("Using FIFO PresentMode");
            swapchain::PresentMode::Fifo
        };

        let (mut swapchain, swapchain_images) = Swapchain::new(
            self.device.clone(),
            surface.clone(),
            SwapchainCreateInfo {
                min_image_count: caps.min_image_count + 1, // How many buffers to use in the swapchain
                image_format,
                image_extent: dimensions.into(),
                image_usage: ImageUsage::COLOR_ATTACHMENT, // What the images are going to be used for
                composite_alpha,
                ..Default::default()
            },
        )
        .unwrap();
        println!("Swapchain Initialized!");

        // println!("{:?}", swapchain);

        self.window = Some(window);
        self.surface = Some(surface);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
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
                self.window.as_ref().unwrap().request_redraw();
            }
            _ => (),
        }
    }
}
