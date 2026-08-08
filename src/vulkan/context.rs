use anyhow::{Context, Result};
use std::sync::Arc;
use vulkano::{
    VulkanLibrary,
    command_buffer::allocator::{CommandBufferAllocator, StandardCommandBufferAllocator},
    device::{Device, Queue, QueueFlags, physical::PhysicalDevice},
    instance::{Instance, InstanceCreateInfo},
    memory::allocator::StandardMemoryAllocator,
};
use winit::event_loop::EventLoop;

use crate::vulkan::system::get_instance_create_info;
/// Shared Vulkan infrastructure: instance, device, queue, allocators.
///
/// Created once during startup and borrowed by all subsystems.
#[derive(Clone)]
pub struct VulkanContext {
    pub instance: Arc<Instance>,
    pub physical_device: Arc<PhysicalDevice>,
    pub device: Arc<Device>,
    pub queue: Arc<Queue>,
    pub memory_allocator: Arc<StandardMemoryAllocator>,
    pub cmd_buffer_allocator: Arc<dyn CommandBufferAllocator>,
}

impl VulkanContext {
    /// Initialise Vulkan: load the library, create an instance, pick a
    /// physical device, create a logical device, and set up allocators.
    ///
    /// `event_loop` is needed to query platform-specific surface extensions.
    pub fn new(event_loop: Option<&EventLoop<()>>) -> Result<Self> {
        let library = VulkanLibrary::new().context("Vulkan loader/dll not found")?;
        let instance_create_info = match event_loop {
            Some(val) => get_instance_create_info(val)?,
            None => InstanceCreateInfo::default(),
        };
        let instance = Instance::new(library, instance_create_info)?;

        let physical_device = instance
            .enumerate_physical_devices()
            .context("could not enumerate devices")?
            .next()
            .context("no devices available")?;

        crate::print_info(&physical_device);

        let queue_family_index = physical_device
            .queue_family_properties()
            .iter()
            .position(|qfp| qfp.queue_flags.contains(QueueFlags::COMPUTE))
            .context("couldn't find a compute queue family")?
            as u32;

        let (device, mut queues) = crate::vulkan::system::create_logical_device(
            physical_device.clone(),
            queue_family_index,
        )?;

        let queue = queues.next().context("no compute queue found")?;

        let memory_allocator = Arc::new(StandardMemoryAllocator::new_default(device.clone()));
        let cmd_buffer_allocator = Arc::new(StandardCommandBufferAllocator::new(
            device.clone(),
            Default::default(),
        ));

        tracing::info!("Vulkan Initialized");

        Ok(Self {
            instance,
            physical_device,
            device,
            queue,
            memory_allocator,
            cmd_buffer_allocator,
        })
    }
}
