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

/// Push constants for the Gaussian blur shader.
///
/// `sigma` controls the standard deviation of the Gaussian kernel.
/// `kernel_size` is the full kernel extent (must be odd; the radius is
/// `kernel_size / 2`).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GaussianBlurInputs {
    pub sigma: f32,
    pub kernel_size: u32,
}

/// Compute pass that applies a 2D Gaussian blur.
///
/// Reads the input texture, writes a blurry version to the output.
pub struct GaussianBlurPass {
    pub pipeline: Arc<ComputePipeline>,
    pub descriptor_set: Arc<DescriptorSet>,
    pub output_image: Arc<Image>,
}

impl GaussianBlurPass {
    /// Build the Gaussian blur pipeline and descriptor set.
    ///
    /// `input_image` is bound at set-0 binding-0; the internally-created
    /// blurred output image is bound at set-0 binding-1.
    pub fn new(ctx: &VulkanContext, input_image: Arc<Image>) -> Result<Self> {
        let output_image = create_output_image(ctx, input_image.extent())?;

        // --- compile SPIR-V ---
        let bytes = include_bytes!(concat!(env!("OUT_DIR"), "/gaussian_blur.spv"));
        let pipeline = create_pipeline(ctx, bytes, "Guassian Blur")?;

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
        inputs: GaussianBlurInputs,
        group_counts: [u32; 3],
    ) -> Result<(), ShaderDispatchError> {
        if inputs.kernel_size.is_multiple_of(2) {
            return Err(ShaderDispatchError::InvalidInput(
                "Kernel size must be odd".to_string(),
            ));
        }
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
