use anyhow::{Error, Result};
use image::DynamicImage;
use std::sync::Arc;
use vulkano::buffer::{Buffer, BufferCreateInfo, BufferUsage};
use vulkano::command_buffer::{
    AutoCommandBufferBuilder, CopyBufferToImageInfo, PrimaryAutoCommandBuffer,
};
use vulkano::format::Format;
use vulkano::image::{Image, ImageCreateInfo, ImageType, ImageUsage};
use vulkano::memory::allocator::{AllocationCreateInfo, MemoryTypeFilter, StandardMemoryAllocator};

pub fn load_image() -> Result<DynamicImage> {
    // Embedded at compile time: on Android there is no project dir at
    // runtime (cwd is `/`), so a relative fs path can never resolve.
    let img = image::load_from_memory(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/moonchill.jpg"
    )))?;
    Ok(img)
}

pub fn upload_image(
    memory_allocator: &Arc<StandardMemoryAllocator>,
    cmd_buffer: &mut AutoCommandBufferBuilder<PrimaryAutoCommandBuffer>,
    img: DynamicImage,
) -> Result<Arc<Image>, Error> {
    let extent = [img.width(), img.height(), 1];

    let upload_buffer = Buffer::from_iter(
        memory_allocator.clone(),
        BufferCreateInfo {
            usage: BufferUsage::TRANSFER_SRC,
            ..Default::default()
        },
        AllocationCreateInfo {
            memory_type_filter: MemoryTypeFilter::PREFER_HOST
                | MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
            ..Default::default()
        },
        img.to_rgba8().into_raw(),
    )?;

    let image = Image::new(
        memory_allocator.clone(),
        ImageCreateInfo {
            image_type: ImageType::Dim2d,
            // UNORM: the compute shader treats texels as raw normalized
            // values (1.0 - v), so no sRGB decode must happen on Load().
            format: Format::R8G8B8A8_UNORM,
            extent,
            usage: ImageUsage::TRANSFER_DST | ImageUsage::SAMPLED,
            ..Default::default()
        },
        AllocationCreateInfo {
            memory_type_filter: MemoryTypeFilter::PREFER_DEVICE,
            ..Default::default()
        },
    )?;

    cmd_buffer.copy_buffer_to_image(CopyBufferToImageInfo::buffer_image(
        upload_buffer,
        image.clone(),
    ))?;
    Ok(image)
}
