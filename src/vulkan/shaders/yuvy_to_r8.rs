use super::errors::ShaderDispatchError;
use crate::vulkan::context::VulkanContext;
use anyhow::{Context, Result};
use std::sync::Arc;
use vulkano::{
    buffer::{Buffer, BufferCreateInfo, BufferUsage, Subbuffer},
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

/// Compute pass that extracts luminance from a YUVY-packed buffer into
/// R8G8B8A8_UNORM.
///
/// The input is a storage buffer of `u32` where each element represents a
/// YUYV 4-byte group: `{ Y_even, U, Y_odd, V }` (byte order).  The output
/// is a full-resolution image with the Y (luminance) value in the R channel
/// (G = B = 0, A = 1).
///
/// No push constants — this is a simple 1:1 mapping.
pub struct YuvyToR8Pass {
    pub pipeline: Arc<ComputePipeline>,
    pub descriptor_set: Arc<DescriptorSet>,
    pub input_buffer: Subbuffer<[u32]>,
    /// Output image (R8G8B8A8_UNORM) with luminance in the R channel.
    pub output_image: Arc<Image>,
}

impl YuvyToR8Pass {
    /// Build the pipeline and descriptor set.
    ///
    /// `pixel_width` and `pixel_height` are the *full* pixel resolution of
    /// the YUYV camera frame — used to size the output image and the
    /// internal storage buffer (which is `pixel_width * pixel_height * 2 / 4`
    /// u32 elements).
    pub fn new(ctx: &VulkanContext, pixel_width: u32, pixel_height: u32) -> Result<Self> {
        let output_extent = [pixel_width, pixel_height, 1];

        let output_image = Image::new(
            ctx.memory_allocator.clone(),
            ImageCreateInfo {
                image_type: ImageType::Dim2d,
                format: Format::R8G8B8A8_UNORM,
                extent: output_extent,
                usage: ImageUsage::STORAGE | ImageUsage::TRANSFER_SRC,
                ..Default::default()
            },
            AllocationCreateInfo {
                memory_type_filter: MemoryTypeFilter::PREFER_DEVICE,
                ..Default::default()
            },
        )?;

        // Storage buffer for raw YUYV data: one u32 per 2 pixels.
        let num_u32 = (pixel_width * pixel_height / 2) as u64;
        let input_buffer: Subbuffer<[u32]> = Buffer::new_slice(
            ctx.memory_allocator.clone(),
            BufferCreateInfo {
                usage: BufferUsage::TRANSFER_DST | BufferUsage::STORAGE_BUFFER,
                ..Default::default()
            },
            AllocationCreateInfo {
                memory_type_filter: MemoryTypeFilter::PREFER_DEVICE,
                ..Default::default()
            },
            num_u32,
        )?;

        // --- compile SPIR-V ---
        let bytes = include_bytes!(concat!(env!("OUT_DIR"), "/yuvy_to_r8.spv"));
        let words =
            bytes_to_words(bytes).context("yuvy_to_r8.spv length is not a multiple of 4")?;
        let module =
            unsafe { ShaderModule::new(ctx.device.clone(), ShaderModuleCreateInfo::new(&words)) }
                .context("failed to create shader module from yuvy_to_r8.spv")?;
        let entry_point = module
            .entry_point("main")
            .context("yuvy_to_r8.spv has no `main` entry point")?;
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
        .context("failed to create YUVY-to-R8 compute pipeline")?;

        // --- descriptor set ---
        // Binding 0: read-only storage buffer (StructuredBuffer<uint>)
        // Binding 1: storage image (RWTexture2D<float4>)
        let ds_allocator = Arc::new(StandardDescriptorSetAllocator::new(
            ctx.device.clone(),
            Default::default(),
        ));
        let descriptor_set = DescriptorSet::new(
            ds_allocator,
            pipeline.layout().set_layouts()[0].clone(),
            [
                WriteDescriptorSet::buffer(0, input_buffer.clone()),
                WriteDescriptorSet::image_view(1, ImageView::new_default(output_image.clone())?),
            ],
            [],
        )?;

        Ok(Self {
            pipeline,
            descriptor_set,
            input_buffer,
            output_image,
        })
    }

    /// Record pipeline bind and compute dispatch.
    pub fn dispatch(
        &self,
        cmd: &mut AutoCommandBufferBuilder<PrimaryAutoCommandBuffer>,
        group_counts: [u32; 3],
    ) -> Result<(), ShaderDispatchError> {
        cmd.bind_pipeline_compute(self.pipeline.clone())?;
        cmd.bind_descriptor_sets(
            PipelineBindPoint::Compute,
            self.pipeline.layout().clone(),
            0,
            self.descriptor_set.clone(),
        )?;
        unsafe { cmd.dispatch(group_counts) }?;
        Ok(())
    }
}
