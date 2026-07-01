use std::sync::Arc;

use vulkano::{
    VulkanLibrary,
    buffer::{Buffer, BufferCreateInfo, BufferUsage},
    command_buffer::{
        AutoCommandBufferBuilder, BlitImageInfo, CommandBufferUsage, CopyBufferToImageInfo,
        allocator::{CommandBufferAllocator, StandardCommandBufferAllocator},
    },
    device::{
        Device, DeviceCreateInfo, DeviceExtensions, Queue, QueueCreateInfo, QueueFlags,
        physical::PhysicalDevice,
    },
    format::Format,
    image::{Image, ImageCreateInfo, ImageType, ImageUsage},
    instance::{Instance, InstanceCreateFlags, InstanceCreateInfo},
    memory::allocator::{AllocationCreateInfo, MemoryTypeFilter, StandardMemoryAllocator},
    swapchain::{self, Surface, SurfaceInfo, Swapchain, SwapchainCreateInfo, SwapchainPresentInfo},
    sync::{self, GpuFuture},
};

use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Window, WindowId},
};

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
    pub fn new(event_loop: &EventLoop<()>) -> Self {
        println!("Starting...");

        let library = VulkanLibrary::new().expect("no local Vulkan library/DLL");

        let required_extensions = Surface::required_extensions(event_loop).unwrap();
        let device_extensions = DeviceExtensions {
            khr_swapchain: true,
            ..Default::default()
        };

        let instance = Instance::new(
            library,
            InstanceCreateInfo {
                flags: InstanceCreateFlags::ENUMERATE_PORTABILITY,
                enabled_extensions: required_extensions,
                ..Default::default()
            },
        )
        .expect("failed to create instance");

        let physical_device = instance
            .enumerate_physical_devices()
            .expect("could not enumerate devices")
            .next()
            .expect("no devices available");

        photon_slam::print_info(&physical_device);

        let queue_family_index = physical_device
            .queue_family_properties()
            .iter()
            .position(|qfp| qfp.queue_flags.contains(QueueFlags::COMPUTE))
            .expect("couldn't find a graphics queue family")
            as u32;

        let (device, mut queues) = Device::new(
            physical_device.clone(),
            DeviceCreateInfo {
                queue_create_infos: vec![QueueCreateInfo {
                    queue_family_index,
                    ..Default::default()
                }],
                enabled_extensions: device_extensions,
                ..Default::default()
            },
        )
        .expect("failed to create device");

        let queue = queues.next().unwrap();

        let memory_allocator = Arc::new(StandardMemoryAllocator::new_default(device.clone()));

        let cmd_buff_allocator = Arc::new(StandardCommandBufferAllocator::new(
            device.clone(),
            Default::default(),
        ));

        println!("Vulkan Initialized");

        let mut uploads = AutoCommandBufferBuilder::primary(
            cmd_buff_allocator.clone(),
            queue_family_index,
            CommandBufferUsage::OneTimeSubmit,
        )
        .unwrap();

        let texture = {
            let img = image::open("cat.jpg").unwrap();
            let extent = [img.width(), img.height(), 1];

            let upload_buffer = Buffer::from_iter(
                memory_allocator.clone(),
                BufferCreateInfo {
                    usage: BufferUsage::TRANSFER_SRC,
                    ..Default::default()
                },
                AllocationCreateInfo {
                    memory_type_filter: MemoryTypeFilter::PREFER_HOST
                        | MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
                    ..Default::default()
                },
                img.to_rgba8().into_raw(),
            )
            .unwrap();

            let image = Image::new(
                memory_allocator.clone(),
                ImageCreateInfo {
                    image_type: ImageType::Dim2d,
                    format: Format::R8G8B8A8_SRGB,
                    extent,
                    usage: ImageUsage::TRANSFER_DST | ImageUsage::TRANSFER_SRC,
                    ..Default::default()
                },
                AllocationCreateInfo {
                    memory_type_filter: MemoryTypeFilter::PREFER_DEVICE,
                    ..Default::default()
                },
            )
            .unwrap();

            uploads
                .copy_buffer_to_image(CopyBufferToImageInfo::buffer_image(
                    upload_buffer,
                    image.clone(),
                ))
                .unwrap();
            // ImageView::new_default(image).unwrap()
            image
        };
        {
            let cmd_buff = uploads.build().unwrap(); // Uploads is builded here
            sync::now(device.clone())
                .then_execute(queue.clone(), cmd_buff)
                .unwrap()
                .flush()
                .unwrap();
            println!("Image uploaded");
        }

        Self {
            instance,
            physical_device,
            device,
            queue,
            image: texture,
            cmd_buffer_allocator: cmd_buff_allocator,
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
