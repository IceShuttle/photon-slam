use crate::vulkan::context::VulkanContext;
use anyhow::{Context, Result};
use std::sync::Arc;
use vulkano::{
    format::Format,
    image::{AllocateImageError, Image, ImageCreateInfo, ImageType, ImageUsage},
    memory::allocator::{AllocationCreateInfo, MemoryTypeFilter},
    pipeline::{
        compute::ComputePipelineCreateInfo,
        layout::{PipelineDescriptorSetLayoutCreateInfo, PipelineLayout},
        ComputePipeline, PipelineShaderStageCreateInfo,
    },
    shader::{spirv::bytes_to_words, ShaderModule, ShaderModuleCreateInfo},
    Validated,
};

pub fn create_output_image(
    ctx: &VulkanContext,
    extent: [u32; 3],
) -> Result<Arc<Image>, Validated<AllocateImageError>> {
    Image::new(
        ctx.memory_allocator.clone(),
        ImageCreateInfo {
            image_type: ImageType::Dim2d,
            format: Format::R8_UNORM,
            extent,
            usage: ImageUsage::STORAGE | ImageUsage::TRANSFER_SRC | ImageUsage::SAMPLED,
            ..Default::default()
        },
        AllocationCreateInfo {
            memory_type_filter: MemoryTypeFilter::PREFER_DEVICE,
            ..Default::default()
        },
    )
}

pub fn create_pipeline(
    ctx: &VulkanContext,
    bytes: &'static [u8],
    pipe_name: &str,
) -> Result<Arc<ComputePipeline>> {
    let words = bytes_to_words(bytes).context("fast.spv length is not a multiple of 4")?;
    let module =
        unsafe { ShaderModule::new(ctx.device.clone(), ShaderModuleCreateInfo::new(&words)) }
            .context(format!("failed to create shader module for {pipe_name}"))?;

    let entry_point = module
        .entry_point("main")
        .context(format!("{pipe_name} has no `main` entry point"))?;

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
    .context(format!("failed to create {pipe_name} compute pipeline"))?;
    Ok(pipeline)
}
