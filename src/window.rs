use std::sync::Arc;

use vulkano::{
    VulkanLibrary,
    buffer::{Buffer, BufferCreateInfo, BufferUsage},
    command_buffer::{
        AutoCommandBufferBuilder, BlitImageInfo, CommandBufferUsage, CopyBufferToImageInfo,
        allocator::{CommandBufferAllocator, StandardCommandBufferAllocator},
    },
    descriptor_set::{
        DescriptorSet, WriteDescriptorSet, allocator::StandardDescriptorSetAllocator,
    },
    device::{
        Device, DeviceCreateInfo, DeviceExtensions, DeviceFeatures, Queue, QueueCreateInfo,
        QueueFlags, physical::PhysicalDevice,
    },
    format::Format,
    image::{Image, ImageCreateInfo, ImageType, ImageUsage, view::ImageView},
    instance::{Instance, InstanceCreateFlags, InstanceCreateInfo},
    memory::allocator::{AllocationCreateInfo, MemoryTypeFilter, StandardMemoryAllocator},
    pipeline::{
        ComputePipeline, Pipeline, PipelineBindPoint, PipelineShaderStageCreateInfo,
        compute::ComputePipelineCreateInfo,
        layout::{PipelineDescriptorSetLayoutCreateInfo, PipelineLayout},
    },
    shader::{ShaderModule, ShaderModuleCreateInfo, spirv::bytes_to_words},
    swapchain::{self, Surface, SurfaceInfo, Swapchain, SwapchainCreateInfo, SwapchainPresentInfo},
    sync::{self, GpuFuture},
};

use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Window, WindowId},
};

pub struct App {
    instance: Arc<Instance>,
    physical_device: Arc<PhysicalDevice>,
    device: Arc<Device>,
    queue: Arc<Queue>,
    compute_pipeline: Arc<ComputePipeline>,
    descriptor_set: Arc<DescriptorSet>,
    output_image: Arc<Image>,
    cmd_buffer_allocator: Arc<dyn CommandBufferAllocator>,
    rcx: Option<RenderContext>,
}

pub struct RenderContext {
    window: Arc<Window>,
    swapchain: Arc<Swapchain>,
    swapchain_images: Vec<Arc<Image>>,
}

impl App {
    pub fn new(event_loop: &EventLoop<()>) -> Self {
        let library = VulkanLibrary::new().expect("no local Vulkan library/DLL");

        let required_extensions = Surface::required_extensions(event_loop).unwrap();
        let device_extensions = DeviceExtensions {
            khr_swapchain: true,
            ..Default::default()
        };

        let instance = Instance::new(
            library,
            InstanceCreateInfo {
                flags: InstanceCreateFlags::ENUMERATE_PORTABILITY,
                enabled_extensions: required_extensions,
                ..Default::default()
            },
        )
        .expect("failed to create instance");

        let physical_device = instance
            .enumerate_physical_devices()
            .expect("could not enumerate devices")
            .next()
            .expect("no devices available");

        photon_slam::print_info(&physical_device);

        let queue_family_index = physical_device
            .queue_family_properties()
            .iter()
            .position(|qfp| qfp.queue_flags.contains(QueueFlags::COMPUTE))
            .expect("couldn't find a graphics queue family")
            as u32;

        assert!(
            physical_device
                .supported_features()
                .shader_storage_image_write_without_format,
            "device lacks shaderStorageImageWriteWithoutFormat, required by compute.spv"
        );

        let (device, mut queues) = Device::new(
            physical_device.clone(),
            DeviceCreateInfo {
                queue_create_infos: vec![QueueCreateInfo {
                    queue_family_index,
                    ..Default::default()
                }],
                enabled_extensions: device_extensions,
                enabled_features: DeviceFeatures {
                    shader_storage_image_write_without_format: true,
                    ..DeviceFeatures::empty()
                },
                ..Default::default()
            },
        )
        .expect("failed to create device");

        let queue = queues.next().unwrap();

        let memory_allocator = Arc::new(StandardMemoryAllocator::new_default(device.clone()));

        let cmd_buff_allocator = Arc::new(StandardCommandBufferAllocator::new(
            device.clone(),
            Default::default(),
        ));

        tracing::info!("Vulkan Initialized");

        let mut uploads = AutoCommandBufferBuilder::primary(
            cmd_buff_allocator.clone(),
            queue_family_index,
            CommandBufferUsage::OneTimeSubmit,
        )
        .unwrap();

        let texture = {
            let img = image::open("cat.jpg").unwrap();
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
            )
            .unwrap();

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
            )
            .unwrap();

            uploads
                .copy_buffer_to_image(CopyBufferToImageInfo::buffer_image(
                    upload_buffer,
                    image.clone(),
                ))
                .unwrap();
            // ImageView::new_default(image).unwrap()
            image
        };
        {
            let cmd_buff = uploads.build().unwrap(); // Uploads is builded here
            sync::now(device.clone())
                .then_execute(queue.clone(), cmd_buff)
                .unwrap()
                .flush()
                .unwrap();
            tracing::debug!("Image uploaded");
        }

        // Storage image the compute shader writes into. Storage images can't be
        // sRGB, so use the UNORM equivalent; same extent as the input texture.
        let output_image = Image::new(
            memory_allocator.clone(),
            ImageCreateInfo {
                image_type: ImageType::Dim2d,
                format: Format::R8G8B8A8_UNORM,
                extent: texture.extent(),
                usage: ImageUsage::STORAGE | ImageUsage::TRANSFER_SRC,
                ..Default::default()
            },
            AllocationCreateInfo {
                memory_type_filter: MemoryTypeFilter::PREFER_DEVICE,
                ..Default::default()
            },
        )
        .unwrap();

        // Cache the compute pipeline: built once here and reused every frame via
        // the stored `Arc`. Only the per-frame dispatch recomputes the output.
        let compute_pipeline = {
            let bytes =
                std::fs::read("shaders/compute.spv").expect("failed to read shaders/compute.spv");
            let words = bytes_to_words(&bytes).expect("compute.spv length is not a multiple of 4");
            let module =
                unsafe { ShaderModule::new(device.clone(), ShaderModuleCreateInfo::new(&words)) }
                    .expect("failed to create shader module from compute.spv");
            let entry_point = module
                .entry_point("main")
                .expect("compute.spv has no `main` entry point");
            let stage = PipelineShaderStageCreateInfo::new(entry_point);
            let layout = PipelineLayout::new(
                device.clone(),
                PipelineDescriptorSetLayoutCreateInfo::from_stages([&stage])
                    .into_pipeline_layout_create_info(device.clone())
                    .unwrap(),
            )
            .unwrap();
            ComputePipeline::new(
                device.clone(),
                None,
                ComputePipelineCreateInfo::stage_layout(stage, layout),
            )
            .expect("failed to create compute pipeline")
        };

        // Bind input (set 0, binding 0) and output (set 0, binding 1) once; the
        // views are stable, so only the dispatch is re-run each redraw.
        let ds_allocator = Arc::new(StandardDescriptorSetAllocator::new(
            device.clone(),
            Default::default(),
        ));
        let descriptor_set = DescriptorSet::new(
            ds_allocator,
            compute_pipeline.layout().set_layouts()[0].clone(),
            [
                WriteDescriptorSet::image_view(0, ImageView::new_default(texture.clone()).unwrap()),
                WriteDescriptorSet::image_view(
                    1,
                    ImageView::new_default(output_image.clone()).unwrap(),
                ),
            ],
            [],
        )
        .unwrap();

        Self {
            instance,
            physical_device,
            device,
            queue,
            compute_pipeline,
            descriptor_set,
            output_image,
            cmd_buffer_allocator: cmd_buff_allocator,
            rcx: None,
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = Arc::new(
            event_loop
                .create_window(Window::default_attributes())
                .unwrap(),
        );
        let surface = Surface::from_window(self.instance.clone(), window.clone()).unwrap();

        let caps = self
            .physical_device
            .surface_capabilities(&surface, Default::default())
            .unwrap();

        let dimensions = window.inner_size();
        let composite_alpha = caps.supported_composite_alpha.into_iter().next().unwrap();

        let formats = self
            .physical_device
            .surface_formats(&surface, SurfaceInfo::default())
            .unwrap();

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
            self.device.clone(),
            surface.clone(),
            SwapchainCreateInfo {
                min_image_count: caps.min_image_count + 1, // How many buffers to use in the swapchain
                image_format,
                image_extent: dimensions.into(),
                image_usage: ImageUsage::COLOR_ATTACHMENT | ImageUsage::TRANSFER_DST, // What the images are going to be used for
                composite_alpha,
                ..Default::default()
            },
        )
        .unwrap();
        tracing::info!("Swapchain Initialized!");
        self.rcx = Some(RenderContext {
            swapchain,
            swapchain_images,
            window,
        });
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                println!("The close button was pressed; stopping");
                event_loop.exit();
            }
            WindowEvent::RedrawRequested => {
                let rcx = self.rcx.as_ref().unwrap();

                let result = swapchain::acquire_next_image(rcx.swapchain.clone(), None);

                let (img_idx, _, acquire_future) = match result {
                    Ok(val) => val,
                    Err(_) => {
                        rcx.window.request_redraw();
                        return;
                    }
                };
                let mut present_builder = AutoCommandBufferBuilder::primary(
                    self.cmd_buffer_allocator.clone(),
                    self.queue.queue_family_index(),
                    CommandBufferUsage::OneTimeSubmit,
                )
                .unwrap();

                let extent = self.output_image.extent();
                let group_counts = [extent[0].div_ceil(16), extent[1].div_ceil(16), 1];

                // Recompute the compute pass every frame; the result is never cached.
                present_builder
                    .bind_pipeline_compute(self.compute_pipeline.clone())
                    .unwrap()
                    .bind_descriptor_sets(
                        PipelineBindPoint::Compute,
                        self.compute_pipeline.layout().clone(),
                        0,
                        self.descriptor_set.clone(),
                    )
                    .unwrap();
                unsafe { present_builder.dispatch(group_counts) }.unwrap();

                present_builder
                    .blit_image(BlitImageInfo::images(
                        self.output_image.clone(),
                        rcx.swapchain_images[img_idx as usize].clone(),
                    ))
                    .unwrap();

                let cmd = match present_builder.build() {
                    Ok(val) => val,
                    Err(_) => return,
                };

                let future = match acquire_future.then_execute(self.queue.clone(), cmd) {
                    Ok(val) => val,
                    Err(_) => return,
                };

                let _ = swapchain::present(
                    future,
                    self.queue.clone(),
                    SwapchainPresentInfo::swapchain_image_index(rcx.swapchain.clone(), img_idx),
                )
                .then_signal_fence_and_flush();

                rcx.window.request_redraw();
            }
            _ => (),
        }
    }
}
