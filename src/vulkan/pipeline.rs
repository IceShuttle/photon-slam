use crate::vulkan::context::VulkanContext;
use anyhow::{Context, Result};
use std::sync::Arc;
use vulkano::{
    command_buffer::{AutoCommandBufferBuilder, PrimaryAutoCommandBuffer},
    descriptor_set::{
        DescriptorSet, WriteDescriptorSet, allocator::StandardDescriptorSetAllocator,
    },
    format::Format,
    image::{Image, ImageCreateInfo, ImageType, ImageUsage, view::ImageView},
    memory::allocator::{AllocationCreateInfo, MemoryTypeFilter},
    pipeline::{
        ComputePipeline, Pipeline, PipelineBindPoint, PipelineShaderStageCreateInfo,
        compute::ComputePipelineCreateInfo,
        layout::{PipelineDescriptorSetLayoutCreateInfo, PipelineLayout},
    },
    shader::{ShaderModule, ShaderModuleCreateInfo, spirv::bytes_to_words},
};

/// Push constants forwarded to the compute shader.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ShaderInputs {
    time: u32,
}

/// Owns the compute pipeline, descriptor set, and output storage image.
///
/// Created once after the input texture is uploaded.  [`Self::dispatch`]
/// records the pipeline-bind, push-constants, descriptor-bind, and
/// dispatch commands into a caller-supplied command buffer.
pub struct ComputePass {
    pub pipeline: Arc<ComputePipeline>,
    pub descriptor_set: Arc<DescriptorSet>,
    pub output_image: Arc<Image>,
}

impl ComputePass {
    /// Build the pipeline and descriptor set from the compiled SPIR-V.
    ///
    /// `input_image` is bound at set-0 binding-0; the internally-created
    /// storage image is bound at set-0 binding-1.
    pub fn new(ctx: &VulkanContext, input_image: Arc<Image>) -> Result<Self> {
        let extent = input_image.extent();

        // --- storage image (written by the compute shader) ---
        let output_image = Image::new(
            ctx.memory_allocator.clone(),
            ImageCreateInfo {
                image_type: ImageType::Dim2d,
                format: Format::R8G8B8A8_UNORM,
                extent,
                usage: ImageUsage::STORAGE | ImageUsage::TRANSFER_SRC,
                ..Default::default()
            },
            AllocationCreateInfo {
                memory_type_filter: MemoryTypeFilter::PREFER_DEVICE,
                ..Default::default()
            },
        )?;

        // --- compile SPIR-V into a pipeline ---
        let bytes = include_bytes!(concat!(env!("OUT_DIR"), "/compute.spv"));
        let words = bytes_to_words(bytes).context("compute.spv length is not a multiple of 4")?;
        let module =
            unsafe { ShaderModule::new(ctx.device.clone(), ShaderModuleCreateInfo::new(&words)) }
                .context("failed to create shader module from compute.spv")?;
        let entry_point = module
            .entry_point("main")
            .context("compute.spv has no `main` entry point")?;
        let stage = PipelineShaderStageCreateInfo::new(entry_point);
        let layout = PipelineLayout::new(
            ctx.device.clone(),
            PipelineDescriptorSetLayoutCreateInfo::from_stages([&stage])
                .into_pipeline_layout_create_info(ctx.device.clone())?,
        )?;
        let pipeline = ComputePipeline::new(
            ctx.device.clone(),
            None,
            ComputePipelineCreateInfo::stage_layout(stage, layout),
        )
        .context("failed to create compute pipeline")?;

        // --- descriptor set (input at 0, output at 1) ---
        let ds_allocator = Arc::new(StandardDescriptorSetAllocator::new(
            ctx.device.clone(),
            Default::default(),
        ));
        let descriptor_set = DescriptorSet::new(
            ds_allocator,
            pipeline.layout().set_layouts()[0].clone(),
            [
                WriteDescriptorSet::image_view(0, ImageView::new_default(input_image)?),
                WriteDescriptorSet::image_view(1, ImageView::new_default(output_image.clone())?),
            ],
            [],
        )?;

        Ok(Self {
            pipeline,
            descriptor_set,
            output_image,
        })
    }

    /// Record pipeline bind, push constants, descriptor binding, and
    /// compute dispatch into `cmd`.
    pub fn dispatch(
        &self,
        cmd: &mut AutoCommandBufferBuilder<PrimaryAutoCommandBuffer>,
        time: u32,
        group_counts: [u32; 3],
    ) -> Result<()> {
        cmd.bind_pipeline_compute(self.pipeline.clone())?;
        cmd.push_constants(self.pipeline.layout().clone(), 0, ShaderInputs { time })?;
        cmd.bind_descriptor_sets(
            PipelineBindPoint::Compute,
            self.pipeline.layout().clone(),
            0,
            self.descriptor_set.clone(),
        )?;
        // SAFETY: group_counts derive from the image extent and match the
        // shader's `[numthreads(16, 16, 1)]` work-group granularity.
        unsafe { cmd.dispatch(group_counts) }?;
        Ok(())
    }
}
