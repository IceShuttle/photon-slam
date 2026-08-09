use super::common::*;
use super::errors::ShaderDispatchError;
use crate::vulkan::context::VulkanContext;
use anyhow::Result;
use std::sync::Arc;
use vulkano::{
    command_buffer::{AutoCommandBufferBuilder, PrimaryAutoCommandBuffer},
    descriptor_set::{
        allocator::StandardDescriptorSetAllocator, DescriptorSet, WriteDescriptorSet,
    },
    image::{view::ImageView, Image},
    pipeline::{ComputePipeline, Pipeline, PipelineBindPoint},
};

/// Compute pass that computes intensity-centroid orientation per pixel.
///
/// Reads luminance from `input_image` (.r channel) and the corner mask
/// from `fast_output` (FAST response image, .r > 0 = corner).  Writes an
/// RGB hue encoding of the orientation angle at each corner pixel; non-
/// corner pixels are black.
pub struct OrbPass {
    pub pipeline: Arc<ComputePipeline>,
    pub descriptor_set: Arc<DescriptorSet>,
    pub output_image: Arc<Image>,
}

impl OrbPass {
    /// Build the ORB orientation pipeline and descriptor set.
    ///
    /// * `input_image` — luminance image (R channel), e.g. YUVY→R8 output.
    /// * `fast_output` — FAST-9 corner response image (.r > 0 → corner).
    pub fn new(
        ctx: &VulkanContext,
        input_image: Arc<Image>,
        fast_output: Arc<Image>,
    ) -> Result<Self> {
        // --- response image (corner score per pixel) ---
        let output_image = create_output_image(ctx, input_image.extent())?;

        // --- compile SPIR-V ---
        let bytes = include_bytes!(concat!(env!("OUT_DIR"), "/orb.spv"));
        let pipeline = create_pipeline(ctx, bytes, "Orb")?;

        // --- descriptor set ---
        // Binding 0: input luminance (Texture2D<float4>)
        // Binding 1: output orientation (RWTexture2D<float4>)
        // Binding 2: FAST corner mask (RWTexture2D<float4>)
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
                WriteDescriptorSet::image_view(2, ImageView::new_default(fast_output)?),
            ],
            [],
        )?;

        Ok(Self {
            pipeline,
            descriptor_set,
            output_image,
        })
    }

    /// Record pipeline bind, descriptor bind, and compute dispatch.
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
