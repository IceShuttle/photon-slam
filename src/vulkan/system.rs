use crate::IS_ANDROID;
use anyhow::{Result};
use vulkano::{
    instance::{InstanceCreateFlags, InstanceCreateInfo, InstanceExtensions},
    swapchain::Surface,
};
use winit::event_loop::EventLoop;

pub fn get_instance_create_info(
    event_loop: &EventLoop<()>,
) -> Result<InstanceCreateInfo> {
    let required_extensions = Surface::required_extensions(event_loop)?;

    let linux_instance_create_info = InstanceCreateInfo {
        flags: InstanceCreateFlags::ENUMERATE_PORTABILITY,
        enabled_extensions: required_extensions,
        ..Default::default()
    };

    let android_instance_create_info = InstanceCreateInfo {
        // ENUMERATE_PORTABILITY is only needed on macOS (MoltenVK);
        // on Android it's meaningless and can confuse the loader.
        // max_api_version capped to 1.0 to avoid vkGetDeviceQueue2
        // which Mali's loader doesn't dispatch properly.
        max_api_version: Some(vulkano::Version {
            major: 1,
            minor: 0,
            patch: 0,
        }),
        enabled_extensions: InstanceExtensions {
            khr_get_physical_device_properties2: true,
            ..required_extensions
        },
        ..Default::default()
    };

    let instance_create_info = match IS_ANDROID {
        true => android_instance_create_info,
        false => linux_instance_create_info,
    };
    
    Ok(instance_create_info)
}
