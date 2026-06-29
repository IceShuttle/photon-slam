use vulkano::device::physical::PhysicalDevice;

pub fn print_info(physical_device: &PhysicalDevice) {
    println!("API Version: {}", physical_device.api_version());
    println!(
        "Device Name:{} and Type:{:?}",
        physical_device.properties().device_name,
        physical_device.properties().device_type
    );
}
