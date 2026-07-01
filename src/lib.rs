use std::sync::Arc;
use vulkano::{
    Validated, VulkanError,
    device::physical::PhysicalDevice,
    swapchain::{PresentMode, Surface, SurfaceInfo},
};

/// Prints the the Vulkan Device and API version
pub fn print_info(physical_device: &PhysicalDevice) {
    println!("API Version: {}", physical_device.api_version());
    println!(
        "Device Name:{} and Type:{:?}",
        physical_device.properties().device_name,
        physical_device.properties().device_type
    );
}

pub fn get_present_mode(
    physical_device: Arc<PhysicalDevice>,
    surface: Arc<Surface>,
) -> Result<PresentMode, Validated<VulkanError>> {
    let present_modes = physical_device.surface_present_modes(&surface, SurfaceInfo::default())?;

    let mode = if present_modes.contains(&PresentMode::Mailbox) {
        println!("Using Mailbox PresentMode");
        PresentMode::Mailbox
    } else if present_modes.contains(&PresentMode::Immediate) {
        println!("Using Immediate PresentMode");
        PresentMode::Immediate
    } else {
        println!("Using FIFO PresentMode");
        PresentMode::Fifo
    };
    Ok(mode)
}
