use crate::vulkan::context::VulkanContext;
use anyhow::{Context, Result};
use std::sync::Arc;
use vulkano::{
    command_buffer::{AutoCommandBufferBuilder, PrimaryAutoCommandBuffer},
    descriptor_set::{
        allocator::StandardDescriptorSetAllocator, DescriptorSet, WriteDescriptorSet,
    },
    format::Format,
    image::{view::ImageView, Image, ImageCreateInfo, ImageType, ImageUsage},
    memory::allocator::{AllocationCreateInfo, MemoryTypeFilter},
    pipeline::{
        compute::ComputePipelineCreateInfo,
        layout::{PipelineDescriptorSetLayoutCreateInfo, PipelineLayout},
        ComputePipeline, Pipeline, PipelineBindPoint, PipelineShaderStageCreateInfo,
    },
    shader::{spirv::bytes_to_words, ShaderModule, ShaderModuleCreateInfo},
};

/// Compute pass that extracts luminance from a YUVY camera image.
///
/// The camera image is an `R8G8B8A8_UNORM` image where each texel stores
/// a YUYV group: `.r=Y0`, `.g=U`, `.b=Y1`, `.a=V`.  The camera image
/// width is half the pixel resolution.  The output is a grayscale R8_UNORM with R=luminance.
///
/// A fresh descriptor set is created every `dispatch()` call so the
/// camera image binding stays current across frames.
pub struct YuvyToR8Pass {
    pub pipeline: Arc<ComputePipeline>,
    ds_allocator: Arc<StandardDescriptorSetAllocator>,
    /// Output image (R8G8B8A8_UNORM) with luminance in the R channel.
    pub output_image: Arc<Image>,
}

impl YuvyToR8Pass {
    /// Build the pipeline and output image.
    ///
    /// `camera_image` is the `R8G8B8A8_UNORM` camera image at width/2
    /// (each texel covers 2 pixels in YUYV packing).  The output is
    /// created at the full pixel resolution (2× the input width).
    pub fn new(ctx: &VulkanContext, camera_image: Arc<Image>) -> Result<Self> {
        let extent = camera_image.extent();
        // Full pixel width = YUYV input width × 2.
        let output_extent = [extent[0] * 2, extent[1], extent[2]];

        let output_image = Image::new(
            ctx.memory_allocator.clone(),
            ImageCreateInfo {
                image_type: ImageType::Dim2d,
                format: Format::R8_UNORM,
                extent: output_extent,
                usage: ImageUsage::STORAGE | ImageUsage::TRANSFER_SRC | ImageUsage::SAMPLED,
                ..Default::default()
            },
            AllocationCreateInfo {
                memory_type_filter: MemoryTypeFilter::PREFER_DEVICE,
                ..Default::default()
            },
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

        let ds_allocator = Arc::new(StandardDescriptorSetAllocator::new(
            ctx.device.clone(),
            Default::default(),
        ));

        Ok(Self {
            pipeline,
            ds_allocator,
            output_image,
        })
    }

    /// Bind the current camera image, bind pipeline, and dispatch.
    ///
    /// `camera_image` must be the same-size `R8G8B8A8_UNORM` camera frame
    /// passed to `new()` — the image data changes per-capture but the
    /// Vulkan `Image` object family is the same.
    pub fn dispatch(
        &self,
        cmd: &mut AutoCommandBufferBuilder<PrimaryAutoCommandBuffer>,
        camera_image: Arc<Image>,
        group_counts: [u32; 3],
    ) -> Result<()> {
        let input_view = ImageView::new_default(camera_image)
            .context("failed to create image view for camera frame")?;
        let output_view = ImageView::new_default(self.output_image.clone())
            .context("failed to create image view for output image")?;

        // Fresh descriptor set per frame so the camera-image binding
        // always points to the latest captured frame.
        let descriptor_set = DescriptorSet::new(
            self.ds_allocator.clone(),
            self.pipeline.layout().set_layouts()[0].clone(),
            [
                WriteDescriptorSet::image_view(0, input_view),
                WriteDescriptorSet::image_view(1, output_view),
            ],
            [],
        )
        .context("failed to create descriptor set for YUVY pass")?;

        cmd.bind_pipeline_compute(self.pipeline.clone())?;
        cmd.bind_descriptor_sets(
            PipelineBindPoint::Compute,
            self.pipeline.layout().clone(),
            0,
            descriptor_set,
        )?;
        unsafe { cmd.dispatch(group_counts) }?;
        Ok(())
    }
}
