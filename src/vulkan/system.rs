use crate::IS_ANDROID;
use anyhow::{Context, Result};
use std::sync::Arc;
use vulkano::device::physical::PhysicalDevice;
use vulkano::device::{
    Device, DeviceCreateInfo, DeviceExtensions, DeviceFeatures, Queue, QueueCreateInfo,
};
use vulkano::{
    instance::{InstanceCreateFlags, InstanceCreateInfo, InstanceExtensions},
    swapchain::Surface,
};
use winit::event_loop::EventLoop;

/// Crates Instance creation info
///
/// Created different info for Android and PC for compatibility
pub fn get_instance_create_info(event_loop: &EventLoop<()>) -> Result<InstanceCreateInfo> {
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
            khr_external_memory_capabilities: true,
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

/// Creates Logical Device
pub fn create_logical_device(
    physical_device: Arc<PhysicalDevice>,
    queue_family_index: u32,
) -> Result<(Arc<Device>, impl ExactSizeIterator<Item = Arc<Queue>>)> {
    Device::new(
        physical_device,
        DeviceCreateInfo {
            queue_create_infos: vec![QueueCreateInfo {
                queue_family_index,
                ..Default::default()
            }],
            enabled_extensions: DeviceExtensions {
                khr_swapchain: true,
                ext_external_memory_dma_buf: true,
                khr_external_memory_fd: true,
                ext_image_drm_format_modifier: false,
                // Needed to import the Camera2 AHardwareBuffer directly as
                // GPU-visible memory (zero-copy) on Android.
                khr_external_memory: IS_ANDROID,
                khr_dedicated_allocation: IS_ANDROID,
                khr_get_memory_requirements2: IS_ANDROID,
                khr_sampler_ycbcr_conversion: IS_ANDROID,
                ext_queue_family_foreign: IS_ANDROID,
                android_external_memory_android_hardware_buffer: IS_ANDROID,
                ..Default::default()
            },
            enabled_features: DeviceFeatures {
                shader_storage_image_write_without_format: true,
                shader_storage_image_read_without_format: IS_ANDROID,
                ..DeviceFeatures::empty()
            },
            ..Default::default()
        },
    )
    .context("failed to create device")
}
