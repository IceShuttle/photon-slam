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

/// Push constants for the FAST-9 shader.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FastInputs {
    pub threshold: f32,
}

/// Compute pass that runs FAST-9 corner detection.
///
/// Reads the input texture, writes a corner-response image.
pub struct FastPass {
    pub pipeline: Arc<ComputePipeline>,
    pub descriptor_set: Arc<DescriptorSet>,
    pub output_image: Arc<Image>,
}

impl FastPass {
    /// Build the FAST pipeline and descriptor set.
    ///
    /// `input_image` is bound at set-0 binding-0; the internally-created
    /// response image is bound at set-0 binding-1.
    pub fn new(ctx: &VulkanContext, input_image: Arc<Image>) -> Result<Self> {
        // --- response image (corner score per pixel) ---
        let output_image = create_output_image(ctx, input_image.extent())?;

        // --- compile SPIR-V ---
        let bytes = include_bytes!(concat!(env!("OUT_DIR"), "/fast.spv"));
        let pipeline = create_pipeline(ctx, bytes, "Fast")?;

        // --- descriptor set ---
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

    /// Record pipeline bind, push constants, and compute dispatch.
    pub fn dispatch(
        &self,
        cmd: &mut AutoCommandBufferBuilder<PrimaryAutoCommandBuffer>,
        inputs: FastInputs,
        group_counts: [u32; 3],
    ) -> Result<(), ShaderDispatchError> {
        cmd.bind_pipeline_compute(self.pipeline.clone())?;
        cmd.push_constants(self.pipeline.layout().clone(), 0, inputs)?;
        cmd.bind_descriptor_sets(
            PipelineBindPoint::Compute,
            self.pipeline.layout().clone(),
            0,
            self.descriptor_set.clone(),
        )?;
        // SAFETY: group_counts derived from image extent; the shader's
        // `[numthreads(16, 16, 1)]` matches the dispatch granularity.
        unsafe { cmd.dispatch(group_counts) }?;
        Ok(())
    }
}
