use image::GenericImageView;
use std::sync::Arc;
use vulkano::buffer::{Buffer, BufferCreateInfo, BufferUsage};
use vulkano::command_buffer::allocator::{
    StandardCommandBufferAllocator, StandardCommandBufferAllocatorCreateInfo,
};
use vulkano::command_buffer::{AutoCommandBufferBuilder, CommandBufferUsage};
use vulkano::command_buffer::{BlitImageInfo, CopyBufferToImageInfo};
use vulkano::device::physical::PhysicalDevice;
use vulkano::device::{Device, DeviceCreateInfo, DeviceExtensions, QueueCreateInfo, QueueFlags};
use vulkano::format::Format;
use vulkano::image::{Image, ImageCreateInfo, ImageType, ImageUsage};
use vulkano::instance::{Instance, InstanceCreateFlags, InstanceCreateInfo, InstanceExtensions};
use vulkano::memory::allocator::{AllocationCreateInfo, MemoryTypeFilter, StandardMemoryAllocator};
use vulkano::swapchain::{
    self, Surface, SurfaceInfo, Swapchain, SwapchainCreateInfo, SwapchainPresentInfo,
};
use vulkano::sync::{self, GpuFuture};
use vulkano::VulkanLibrary;
use winit::event::{Event, WindowEvent};
use winit::event_loop::EventLoop;
use winit::window::WindowAttributes;

fn print_info(physical_device: &PhysicalDevice) {
    println!("API Version: {}", physical_device.api_version());
    println!(
        "Device Name:{} and Type:{:?}",
        physical_device.properties().device_name,
        physical_device.properties().device_type
    );
}

fn main() {
    println!("Starting...");

    let library = VulkanLibrary::new().expect("no local Vulkan library/DLL");
    let event_loop = EventLoop::new().expect("Cannot create winit init loop");

    let surface_extensions = Surface::required_extensions(&event_loop).unwrap();
    let instance_extensions = InstanceExtensions {
        khr_surface: true,
        ..surface_extensions
    };

    let instance = Instance::new(
        library,
        InstanceCreateInfo {
            flags: InstanceCreateFlags::ENUMERATE_PORTABILITY,
            enabled_extensions: instance_extensions,
            ..Default::default()
        },
    )
    .expect("failed to create instance");
    let physical_device = instance
        .enumerate_physical_devices()
        .expect("could not enumerate devices")
        .next()
        .expect("no devices available");

    let queue_family_index = physical_device
        .queue_family_properties()
        .iter()
        .position(|qfp| qfp.queue_flags.contains(QueueFlags::GRAPHICS))
        .expect("couldn't find a graphics queue family") as u32;

    let device_extensions = DeviceExtensions {
        khr_swapchain: true,
        ..DeviceExtensions::empty()
    };

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

    println!("Vulkan Initialized");
    print_info(&physical_device);

    let window = Arc::new(
        event_loop
            .create_window(WindowAttributes::default().with_title("cat"))
            .unwrap(),
    );

    let surface = Surface::from_window(instance.clone(), window.clone()).unwrap();

    let caps = physical_device
        .surface_capabilities(&surface, SurfaceInfo::default())
        .unwrap();
    let formats = physical_device
        .surface_formats(&surface, SurfaceInfo::default())
        .unwrap();

    let (image_format, _) = match formats.iter().find(|(fmt, _)| {
        matches!(
            *fmt,
            Format::B8G8R8A8_SRGB
                | Format::R8G8B8A8_SRGB
                | Format::B8G8R8A8_UNORM
                | Format::R8G8B8A8_UNORM
        )
    }) {
        Some(f) => *f,
        None => formats[0],
    };

    let present_modes = physical_device
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

    let window_size = window.inner_size();
    let image_extent = [window_size.width.max(1), window_size.height.max(1)];

    let composite_alpha = caps.supported_composite_alpha.into_iter().next().unwrap();

    let (swapchain, swapchain_images) = Swapchain::new(
        device.clone(),
        surface.clone(),
        SwapchainCreateInfo {
            min_image_count: caps.min_image_count.max(2),
            image_format,
            image_extent,
            image_usage: ImageUsage::TRANSFER_DST,
            composite_alpha,
            present_mode,
            ..Default::default()
        },
    )
    .unwrap();

    let memory_allocator = Arc::new(StandardMemoryAllocator::new_default(device.clone()));
    let command_buffer_allocator = Arc::new(StandardCommandBufferAllocator::new(
        device.clone(),
        StandardCommandBufferAllocatorCreateInfo::default(),
    ));

    // Load JPEG and upload to GPU
    let img = image::open("cat.jpg").expect("failed to load cat.jpg");
    let (width, height) = img.dimensions();
    let rgba = img.to_rgba8().into_raw(); // Converting compressed image to raw RGBA channel

    let staging_buffer = Buffer::from_iter(
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
        rgba,
    )
    .expect("failed to create staging buffer");

    let cat_image = Image::new(
        memory_allocator.clone(),
        ImageCreateInfo {
            image_type: ImageType::Dim2d,
            format: Format::R8G8B8A8_SRGB,
            extent: [width, height, 1],
            usage: ImageUsage::TRANSFER_DST | ImageUsage::TRANSFER_SRC,
            ..Default::default()
        },
        AllocationCreateInfo {
            memory_type_filter: MemoryTypeFilter::PREFER_DEVICE,
            ..Default::default()
        },
    )
    .expect("failed to create image");

    let mut builder = AutoCommandBufferBuilder::primary(
        command_buffer_allocator.clone(),
        queue_family_index,
        CommandBufferUsage::OneTimeSubmit,
    )
    .unwrap();

    builder
        .copy_buffer_to_image(CopyBufferToImageInfo::buffer_image(
            staging_buffer,
            cat_image.clone(),
        ))
        .unwrap();

    let upload_cmd = builder.build().unwrap();

    sync::now(device.clone())
        .then_execute(queue.clone(), upload_cmd)
        .unwrap()
        .flush()
        .unwrap();

    println!("Image Uploaded");

    // Render loop
    let _ = event_loop.run(move |event, target| match event {
        Event::WindowEvent {
            event: WindowEvent::RedrawRequested,
            ..
        } => {
            let result = swapchain::acquire_next_image(swapchain.clone(), None);
            let (image_index, _, acquire_future) = match result {
                Ok(v) => v,
                Err(_) => {
                    window.request_redraw();
                    return;
                }
            };

            let mut builder = AutoCommandBufferBuilder::primary(
                command_buffer_allocator.clone(),
                queue_family_index,
                CommandBufferUsage::OneTimeSubmit,
            )
            .unwrap();

            builder
                .blit_image(BlitImageInfo::images(
                    cat_image.clone(),
                    swapchain_images[image_index as usize].clone(),
                ))
                .unwrap();

            let cmd = match builder.build() {
                Ok(c) => c,
                Err(_) => return,
            };

            let future = match acquire_future.then_execute(queue.clone(), cmd) {
                Ok(f) => f,
                Err(_) => return,
            };

            let _ = swapchain::present(
                future,
                queue.clone(),
                SwapchainPresentInfo::swapchain_image_index(swapchain.clone(), image_index),
            )
            .then_signal_fence_and_flush();

            window.request_redraw();
        }
        Event::WindowEvent {
            event: WindowEvent::CloseRequested,
            ..
        } => {
            target.exit();
        }
        _ => {}
    });
}
