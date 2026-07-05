use anyhow::{Context, Result};
use std::sync::Arc;
use vulkano::device::Device;
use vulkano::device::physical::PhysicalDevice;
use vulkano::{
    format::Format,
    image::{Image, ImageUsage},
    instance::Instance,
    swapchain::{Surface, SurfaceInfo, Swapchain, SwapchainCreateInfo},
};
use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

/// Contains Everything required to draw on Screen
pub struct RenderContext {
    pub window: Arc<Window>,
    pub swapchain: Arc<Swapchain>,
    pub swapchain_images: Vec<Arc<Image>>,
}
impl RenderContext {
    /// Crates a new rendering context
    ///
    /// Uses UNORM in display format instead of sRGB for simple image manipulation
    pub fn new(
        event_loop: &ActiveEventLoop,
        instance: Arc<Instance>,
        physical_device: Arc<PhysicalDevice>,
        device: Arc<Device>,
    ) -> Result<Self> {
        let window = Arc::new(event_loop.create_window(Window::default_attributes())?);
        let surface = Surface::from_window(instance.clone(), window.clone())?;

        let caps = physical_device.surface_capabilities(&surface, Default::default())?;

        let dimensions = window.inner_size();
        let composite_alpha = caps
            .supported_composite_alpha
            .into_iter()
            .next()
            .context("No composite alpha supported")?;

        let formats = physical_device.surface_formats(&surface, SurfaceInfo::default())?;

        tracing::debug!("Formats available: {:?}", formats);

        let image_format = match formats
            .iter()
            .find(|(fmt, _)| matches!(*fmt, Format::B8G8R8A8_UNORM | Format::R8G8B8A8_UNORM))
        {
            Some(f) => f.0,
            None => {
                tracing::info!("Using format {:?}", &formats[0].0);
                formats[0].0
            }
        };
        tracing::debug!("Selected Format: {:?}", image_format);

        let (swapchain, swapchain_images) = Swapchain::new(
            device.clone(),
            surface.clone(),
            SwapchainCreateInfo {
                min_image_count: caps.min_image_count + 1, // How many buffers to use in the swapchain
                image_format,
                image_extent: dimensions.into(),
                image_usage: ImageUsage::COLOR_ATTACHMENT | ImageUsage::TRANSFER_DST, // What the images are going to be used for
                composite_alpha,
                ..Default::default()
            },
        )?;
        tracing::info!("Swapchain Initialized!");
        Ok(Self {
            window,
            swapchain,
            swapchain_images,
        })
    }
}
