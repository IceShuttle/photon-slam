use crate::{
    fps::FpsCounter,
    vulkan::{self, disp::RenderContext},
};
use anyhow::{Context, Result};
use std::{sync::Arc, time::SystemTime};

use vulkano::{
    VulkanLibrary,
    command_buffer::{
        AutoCommandBufferBuilder, BlitImageInfo, CommandBufferUsage,
        allocator::{CommandBufferAllocator, StandardCommandBufferAllocator},
    },
    descriptor_set::{
        DescriptorSet, WriteDescriptorSet, allocator::StandardDescriptorSetAllocator,
    },
    device::{Device, Queue, QueueFlags, physical::PhysicalDevice},
    format::Format,
    image::{Image, ImageCreateInfo, ImageType, ImageUsage, view::ImageView},
    instance::Instance,
    memory::allocator::{AllocationCreateInfo, MemoryTypeFilter, StandardMemoryAllocator},
    pipeline::{
        ComputePipeline, Pipeline, PipelineBindPoint, PipelineShaderStageCreateInfo,
        compute::ComputePipelineCreateInfo,
        layout::{PipelineDescriptorSetLayoutCreateInfo, PipelineLayout},
    },
    shader::{ShaderModule, ShaderModuleCreateInfo, spirv::bytes_to_words},
    swapchain::{self, SwapchainPresentInfo},
    sync::{self, GpuFuture},
};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    window::WindowId,
};

pub struct App {
    instance: Arc<Instance>,
    physical_device: Arc<PhysicalDevice>,
    device: Arc<Device>,
    queue: Arc<Queue>,
    compute_pipeline: Arc<ComputePipeline>,
    descriptor_set: Arc<DescriptorSet>,
    output_image: Arc<Image>,
    cmd_buffer_allocator: Arc<dyn CommandBufferAllocator>,
    rcx: Option<RenderContext>,
    fps: FpsCounter,
    start_time: SystemTime,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ShaderInputs {
    time: u32,
}

impl App {
    pub fn new(event_loop: &EventLoop<()>) -> Result<Self> {
        let library = VulkanLibrary::new().context("Vulkan loader/dll not found")?;
        let instance_create_info = vulkan::system::get_instance_create_info(event_loop)?;
        let instance = Instance::new(library, instance_create_info)?;
        let physical_device = instance
            .enumerate_physical_devices()
            .expect("could not enumerate devices")
            .next()
            .expect("no devices available");

        crate::print_info(&physical_device);

        let queue_family_index = physical_device
            .queue_family_properties()
            .iter()
            .position(|qfp| qfp.queue_flags.contains(QueueFlags::COMPUTE))
            .expect("couldn't find a graphics queue family")
            as u32;

        let (device, mut queues) =
            vulkan::system::create_logical_device(physical_device.clone(), queue_family_index)?;

        let queue = queues.next().context("no graphics queue found")?;

        let memory_allocator = Arc::new(StandardMemoryAllocator::new_default(device.clone()));

        let cmd_buff_allocator = Arc::new(StandardCommandBufferAllocator::new(
            device.clone(),
            Default::default(),
        ));

        tracing::info!("Vulkan Initialized");

        let mut uploads = AutoCommandBufferBuilder::primary(
            cmd_buff_allocator.clone(),
            queue_family_index,
            CommandBufferUsage::OneTimeSubmit,
        )?;

        let img = vulkan::image::load_image()?;
        let texture = vulkan::image::upload_image(&memory_allocator, &mut uploads, img)?;
        {
            let cmd_buff = uploads.build()?; // Uploads is builded here
            sync::now(device.clone())
                .then_execute(queue.clone(), cmd_buff)?
                .flush()?;
            tracing::debug!("Image uploaded");
        }

        // Storage image the compute shader writes into. Storage images can't be
        // sRGB, so use the UNORM equivalent; same extent as the input texture.
        let output_image = Image::new(
            memory_allocator.clone(),
            ImageCreateInfo {
                image_type: ImageType::Dim2d,
                format: Format::R8G8B8A8_UNORM,
                extent: texture.extent(),
                usage: ImageUsage::STORAGE | ImageUsage::TRANSFER_SRC,
                ..Default::default()
            },
            AllocationCreateInfo {
                memory_type_filter: MemoryTypeFilter::PREFER_DEVICE,
                ..Default::default()
            },
        )?;

        // Cache the compute pipeline: built once here and reused every frame via
        // the stored `Arc`. Only the per-frame dispatch recomputes the output.
        let compute_pipeline = {
            let bytes = include_bytes!(concat!(env!("OUT_DIR"), "/compute.spv"));
            let words = bytes_to_words(bytes).expect("compute.spv length is not a multiple of 4");
            let module =
                unsafe { ShaderModule::new(device.clone(), ShaderModuleCreateInfo::new(&words)) }
                    .context("failed to create shader module from compute.spv")?;
            let entry_point = module
                .entry_point("main")
                .context("compute.spv has no `main` entry point")?;
            let stage = PipelineShaderStageCreateInfo::new(entry_point);
            let layout = PipelineLayout::new(
                device.clone(),
                PipelineDescriptorSetLayoutCreateInfo::from_stages([&stage])
                    .into_pipeline_layout_create_info(device.clone())?,
            )?;
            ComputePipeline::new(
                device.clone(),
                None,
                ComputePipelineCreateInfo::stage_layout(stage, layout),
            )
            .context("failed to create compute pipeline")?
        };

        // Bind input (set 0, binding 0) and output (set 0, binding 1) once; the
        // views are stable, so only the dispatch is re-run each redraw.
        let ds_allocator = Arc::new(StandardDescriptorSetAllocator::new(
            device.clone(),
            Default::default(),
        ));
        let descriptor_set = DescriptorSet::new(
            ds_allocator,
            compute_pipeline.layout().set_layouts()[0].clone(),
            [
                WriteDescriptorSet::image_view(0, ImageView::new_default(texture.clone())?),
                WriteDescriptorSet::image_view(1, ImageView::new_default(output_image.clone())?),
            ],
            [],
        )?;

        Ok(Self {
            instance,
            physical_device,
            device,
            queue,
            compute_pipeline,
            descriptor_set,
            output_image,
            cmd_buffer_allocator: cmd_buff_allocator,
            rcx: None,
            fps: FpsCounter::new(),
            start_time: SystemTime::now(),
        })
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let rcx = RenderContext::new(
            event_loop,
            self.instance.clone(),
            self.physical_device.clone(),
            self.device.clone(),
        )
        .ok();
        self.rcx = rcx;
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                tracing::info!("The close button was pressed; stopping");
                event_loop.exit();
            }
            WindowEvent::RedrawRequested => {
                let rcx = self.rcx.as_ref().unwrap();

                let result = swapchain::acquire_next_image(rcx.swapchain.clone(), None);

                let mut present_builder = AutoCommandBufferBuilder::primary(
                    self.cmd_buffer_allocator.clone(),
                    self.queue.queue_family_index(),
                    CommandBufferUsage::OneTimeSubmit,
                )
                .unwrap();

                let extent = self.output_image.extent();
                let group_counts = [extent[0].div_ceil(16), extent[1].div_ceil(16), 1];
                let curr_time = SystemTime::now();
                let elapsed = curr_time
                    .duration_since(self.start_time)
                    .unwrap()
                    .as_millis() as u32;
                // Recompute the compute pass every frame; the result is never cached.
                present_builder
                    .bind_pipeline_compute(self.compute_pipeline.clone())
                    .unwrap()
                    .push_constants(
                        self.compute_pipeline.layout().clone(),
                        0,
                        ShaderInputs { time: elapsed },
                    )
                    .unwrap()
                    .bind_descriptor_sets(
                        PipelineBindPoint::Compute,
                        self.compute_pipeline.layout().clone(),
                        0,
                        self.descriptor_set.clone(),
                    )
                    .unwrap();
                unsafe { present_builder.dispatch(group_counts) }.unwrap();

                let (img_idx, _, acquire_future) = match result {
                    Ok(val) => val,
                    Err(_) => {
                        rcx.window.request_redraw();
                        return;
                    }
                };
                present_builder
                    .blit_image(BlitImageInfo::images(
                        self.output_image.clone(),
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

                self.fps.tick();

                rcx.window.request_redraw();
            }
            _ => (),
        }
    }
}
