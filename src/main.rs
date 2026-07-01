use std::sync::Arc;

use image::ImageReader;
use vulkano::buffer::{Buffer, BufferContents, BufferCreateInfo, BufferUsage, Subbuffer};
use vulkano::command_buffer::{
    AutoCommandBufferBuilder, CommandBufferUsage, CopyBufferToImageInfo,
    allocator::StandardCommandBufferAllocator,
};
use vulkano::descriptor_set::allocator::StandardDescriptorSetAllocator;
use vulkano::device::{Device, DeviceCreateInfo, DeviceExtensions, QueueCreateInfo, QueueFlags};
use vulkano::format::Format;
use vulkano::image::{
    Image, ImageCreateInfo, ImageType, ImageUsage,
    sampler::{Filter, Sampler, SamplerAddressMode, SamplerCreateInfo},
    view::ImageView,
};
use vulkano::instance::{Instance, InstanceCreateFlags, InstanceCreateInfo};
use vulkano::memory::allocator::{AllocationCreateInfo, MemoryTypeFilter, StandardMemoryAllocator};
use vulkano::pipeline::graphics::vertex_input::Vertex;
use vulkano::swapchain::Surface;
use vulkano::sync::{self, GpuFuture};
use vulkano::{DeviceSize, VulkanLibrary};
use winit::event_loop::{self, EventLoop};
mod window;

#[derive(BufferContents, Vertex)]
#[repr(C)]
struct Vertex2D {
    #[format(R32G32_SFLOAT)]
    position: [f32; 2],
}

fn main() {
    println!("Starting...");

    let library = VulkanLibrary::new().expect("no local Vulkan library/DLL");
    let event_loop = EventLoop::new().unwrap();

    let required_extensions = Surface::required_extensions(&event_loop).unwrap();
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
        .position(|qfp| {
            qfp.queue_flags
                .contains(QueueFlags::GRAPHICS & QueueFlags::COMPUTE)
        })
        .expect("couldn't find a graphics queue family") as u32;

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
    let desc_set_allocator = Arc::new(StandardDescriptorSetAllocator::new(
        device.clone(),
        Default::default(),
    ));
    let cmd_buff_allocator = Arc::new(StandardCommandBufferAllocator::new(
        device.clone(),
        Default::default(),
    ));

    println!("Vulkan Initialized");

    let vertices = [
        Vertex2D {
            position: [-1.0, 1.0],
        },
        Vertex2D {
            position: [3.0, 1.0],
        },
        Vertex2D {
            position: [-1.0, -3.0],
        },
    ];

    let vertex_buffer = Buffer::from_iter(
        memory_allocator.clone(),
        BufferCreateInfo {
            usage: BufferUsage::VERTEX_BUFFER,
            ..Default::default()
        },
        AllocationCreateInfo {
            memory_type_filter: MemoryTypeFilter::PREFER_DEVICE
                | MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
            ..Default::default()
        },
        vertices,
    )
    .unwrap();
    println!("Created Vertex Buffer");

    let mut uploads = AutoCommandBufferBuilder::primary(
        cmd_buff_allocator.clone(),
        queue_family_index,
        CommandBufferUsage::OneTimeSubmit,
    )
    .unwrap();

    let texture = {
        let img = ImageReader::open("cat.jpg").unwrap().decode().unwrap();
        let buffer_size = (img.width() * img.height() * 4) as DeviceSize;
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
                extent: [img.width(), img.height(), 1],
                usage: ImageUsage::TRANSFER_DST | ImageUsage::TRANSFER_SRC | ImageUsage::SAMPLED,
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
        ImageView::new_default(image).unwrap()
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
    // let sampler = Sampler::new(
    //     device.clone(),
    //     SamplerCreateInfo {
    //         mag_filter: Filter::Linear,
    //         min_filter: Filter::Linear,
    //         address_mode: [SamplerAddressMode::Repeat; 3],
    //         ..Default::default()
    //     },
    // )
    // .unwrap();
    // println!("{:?}", texture.image().memory());
    let mut app = window::App::new(
        instance,
        physical_device,
        device,
        queue,
        texture,
        desc_set_allocator,
        None,
        None,
        cmd_buff_allocator,
    );
    event_loop.set_control_flow(event_loop::ControlFlow::Poll);
    event_loop.run_app(&mut app).unwrap();
}
