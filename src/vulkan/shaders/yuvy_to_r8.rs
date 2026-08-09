use anyhow::{Context, Result};
use std::sync::Arc;
use vulkano::device::DeviceOwned;
use vulkano::{
    command_buffer::{AutoCommandBufferBuilder, PrimaryAutoCommandBuffer},
    descriptor_set::{
        allocator::StandardDescriptorSetAllocator, DescriptorSet, WriteDescriptorSet,
    },
    format::Format,
    image::{
        sampler::{
            ycbcr::SamplerYcbcrConversion, ComponentMapping, Filter, Sampler, SamplerCreateInfo,
        },
        view::{ImageView, ImageViewCreateInfo},
        Image, ImageAspects, ImageSubresourceRange, ImageUsage,
    },
    pipeline::{
        compute::ComputePipelineCreateInfo,
        layout::{PipelineDescriptorSetLayoutCreateInfo, PipelineLayout},
        ComputePipeline, Pipeline, PipelineBindPoint, PipelineShaderStageCreateInfo,
    },
    shader::{spirv::bytes_to_words, ShaderModule, ShaderModuleCreateInfo},
};

use crate::vulkan::context::VulkanContext;

use super::common::create_output_image;

/// Compute pass that extracts luminance from a camera image.
///
/// Two input layouts are supported, selected by whether `ycbcr_conversion`
/// was given to [`new`](Self::new):
/// - Linux (`None`): the camera image is an `R8G8B8A8_UNORM` storage image
///   where each texel stores a YUYV group (`.r=Y0`, `.g=U`, `.b=Y1`,
///   `.a=V`) at half the pixel resolution — unpacked by hand in the shader.
/// - Android (`Some`): the camera image has an opaque, vendor-defined
///   pixel format and is imported zero-copy from an `AHardwareBuffer`; it
///   can only be read via a combined image sampler with an immutable
///   YCbCr-identity conversion, which hands back raw luminance in `.g`
///   (per the Vulkan multi-planar format spec: G=Y, B=Cb, R=Cr) directly
///   at full resolution — no unpacking needed.
///
/// Either way the output is a full-resolution grayscale `R8_UNORM` image
/// with `R=luminance`.
///
/// A fresh descriptor set is created every `dispatch()` call so the
/// camera image binding stays current across frames.
pub struct YuvyToR8Pass {
    pub pipeline: Arc<ComputePipeline>,
    ds_allocator: Arc<StandardDescriptorSetAllocator>,
    ycbcr_conversion: Option<Arc<SamplerYcbcrConversion>>,
    /// Output image (R8_UNORM) with luminance in the R channel.
    pub output_image: Arc<Image>,
}

impl YuvyToR8Pass {
    /// Build the pipeline and output image.
    ///
    /// `camera_image` is the platform camera frame (see struct docs for
    /// its two possible layouts). `ycbcr_conversion` must be `Some` on
    /// Android (derived from the imported hardware buffer's format
    /// properties) and `None` on Linux.
    pub fn new(
        ctx: &VulkanContext,
        camera_image: Arc<Image>,
        ycbcr_conversion: Option<Arc<SamplerYcbcrConversion>>,
    ) -> Result<Self> {
        let extent = camera_image.extent();
        // Linux packs 2 pixels per input texel (half-width YUYV); Android
        // imports the camera image at full pixel resolution already.
        let output_extent = match &ycbcr_conversion {
            Some(_) => extent,
            None => [extent[0] * 2, extent[1], extent[2]],
        };

        let output_image = create_output_image(ctx, output_extent)?;

        // --- compile SPIR-V ---
        let bytes = match &ycbcr_conversion {
            Some(_) => {
                include_bytes!(concat!(env!("OUT_DIR"), "/android_yuvy_to_r8.spv")).as_slice()
            }
            None => include_bytes!(concat!(env!("OUT_DIR"), "/yuvy_to_r8.spv")).as_slice(),
        };
        let words =
            bytes_to_words(bytes).context("yuvy_to_r8 shader length is not a multiple of 4")?;
        let module =
            unsafe { ShaderModule::new(ctx.device.clone(), ShaderModuleCreateInfo::new(&words)) }
                .context("failed to create shader module for yuvy_to_r8")?;
        let entry_point = module
            .entry_point("main")
            .context("yuvy_to_r8 shader has no `main` entry point")?;
        let stage = PipelineShaderStageCreateInfo::new(entry_point);

        let mut layout_info = PipelineDescriptorSetLayoutCreateInfo::from_stages([&stage]);
        // Android's combined image sampler (binding 0) needs its
        // conversion baked into the layout as an immutable sampler —
        // sampler YCbCr conversions can't be set per-descriptor-write.
        if let Some(conversion) = &ycbcr_conversion {
            let sampler = Sampler::new(
                ctx.device.clone(),
                SamplerCreateInfo {
                    mag_filter: Filter::Linear,
                    min_filter: Filter::Linear,
                    sampler_ycbcr_conversion: Some(conversion.clone()),
                    ..Default::default()
                },
            )
            .context("failed to create YCbCr-conversion sampler")?;
            layout_info.set_layouts[0]
                .bindings
                .get_mut(&0)
                .context("yuvy_to_r8 shader has no binding 0")?
                .immutable_samplers = vec![sampler];
        }

        let layout = PipelineLayout::new(
            ctx.device.clone(),
            layout_info.into_pipeline_layout_create_info(ctx.device.clone())?,
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
            ycbcr_conversion,
            output_image,
        })
    }

    /// Bind the current camera image, bind pipeline, and dispatch.
    ///
    /// `camera_image` must be the same-family camera frame passed to
    /// `new()` — the image data changes per-capture but its format/layout
    /// does not.
    pub fn dispatch(
        &self,
        cmd: &mut AutoCommandBufferBuilder<PrimaryAutoCommandBuffer>,
        camera_image: Arc<Image>,
        group_counts: [u32; 3],
    ) -> Result<()> {
        let input_view = match &self.ycbcr_conversion {
            Some(conversion) => create_ycbcr_view(camera_image, conversion)
                .context("failed to create YCbCr image view for camera frame")?,
            None => ImageView::new_default(camera_image)
                .context("failed to create image view for camera frame")?,
        };
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

/// Creates an image view for an opaque (`format = UNDEFINED`) external
/// Android format image, with `conversion` attached.
///
/// Vulkano's safe `ImageView::new`/`new_unchecked` bakes its
/// `ImageViewCreateInfo::format` field straight into the real
/// `vkCreateImageView` call. That call must use `VK_FORMAT_UNDEFINED` here
/// (required whenever a `VkSamplerYcbcrConversionInfo` with a nonzero
/// external format is chained), but vulkano's own *bookkeeping* then
/// treats the view as having no color aspect / empty format features,
/// which panics elsewhere (format_features validation, and
/// `numeric_format_color().unwrap()` in dispatch-time descriptor
/// checking). So the real call is made with raw Vulkan calls, and the
/// result is wrapped with a placeholder concrete bookkeeping format
/// (`R8G8B8A8_UNORM`) that vulkano's other code paths can reason about.
fn create_ycbcr_view(
    camera_image: Arc<Image>,
    conversion: &Arc<SamplerYcbcrConversion>,
) -> Result<Arc<ImageView>> {
    use ash::vk;
    use vulkano::VulkanObject;

    let device = camera_image.device().clone();
    let mut ycbcr_info = vk::SamplerYcbcrConversionInfo::default().conversion(conversion.handle());
    let create_info_vk = vk::ImageViewCreateInfo::default()
        .image(camera_image.handle())
        .view_type(vk::ImageViewType::TYPE_2D)
        .format(vk::Format::UNDEFINED)
        .components(vk::ComponentMapping::default())
        .subresource_range(vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        })
        .push_next(&mut ycbcr_info);

    let mut handle = vk::ImageView::null();
    let status = unsafe {
        (device.fns().v1_0.create_image_view)(
            device.handle(),
            &create_info_vk,
            std::ptr::null(),
            &mut handle,
        )
    };
    if status != vk::Result::SUCCESS {
        anyhow::bail!("vkCreateImageView (YCbCr) failed: {status:?}");
    }

    // SAFETY: `handle` was just created from `camera_image`'s device;
    // `format`/`sampler_ycbcr_conversion` below are bookkeeping-only (see
    // function doc) and don't feed back into a real Vulkan call.
    Ok(unsafe {
        ImageView::from_handle(
            camera_image,
            handle,
            ImageViewCreateInfo {
                format: Format::R8G8B8A8_UNORM,
                component_mapping: ComponentMapping::identity(),
                subresource_range: ImageSubresourceRange {
                    aspects: ImageAspects::COLOR,
                    mip_levels: 0..1,
                    array_layers: 0..1,
                },
                usage: ImageUsage::SAMPLED,
                sampler_ycbcr_conversion: Some(conversion.clone()),
                ..Default::default()
            },
        )
    }?)
}
