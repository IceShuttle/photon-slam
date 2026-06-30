use vulkano::VulkanLibrary;
use vulkano::device::{Device, DeviceCreateInfo, DeviceExtensions, QueueCreateInfo, QueueFlags};
use vulkano::instance::{Instance, InstanceCreateFlags, InstanceCreateInfo};
use vulkano::swapchain::Surface;
use winit::event_loop::{self, EventLoop};
mod window;

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
    println!("Vulkan Initialized");

    photon_slam::print_info(&physical_device);

    event_loop.set_control_flow(event_loop::ControlFlow::Poll);
    let mut app = window::App::new(instance, physical_device, device, queue);
    event_loop.run_app(&mut app).unwrap();
}
