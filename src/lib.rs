//! # Photon SLAM
//!
//! An GPU acclerated implementation of vSLAM that aims works on both Linux and Android through use of Vulkan
//!
//! It tries to achieve this using the vulkan compute and shaders written in slang
//! It Currently uses winit for window management
/// Frame-rate counter using `tracing`.
pub mod fps;
/// Vulkan QOL utilities for instantiation and boilerplate
pub mod vulkan;
/// Creates Window for Display
pub mod window;

/// General Utils
pub mod utils;

#[cfg(target_os = "android")]
use winit::platform::android::activity::AndroidApp;

#[cfg(target_os = "android")]
use winit::{
    event_loop::{ControlFlow, EventLoop},
    platform::android::EventLoopBuilderExtAndroid,
};

/// Tells wether it is being compiled for android or not!
pub const IS_ANDROID: bool = cfg!(target_os = "android");
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    use window::App;
    // Route `tracing` (via its `log` feature) and `log` records to logcat.
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Debug)
            .with_tag("photon-slam"),
    );
    // android-activity aborts on unwind; log the panic first so logcat shows
    // the actual message instead of a bare native crash.
    std::panic::set_hook(Box::new(|info| {
        log::error!("panic: {info}");
    }));

    let event_loop = EventLoop::with_user_event()
        .with_android_app(app)
        .build()
        .unwrap();

    let mut app = App::new(&event_loop).unwrap();
    event_loop.set_control_flow(ControlFlow::Poll);
    event_loop.run_app(&mut app).unwrap();
}

use vulkano::device::physical::PhysicalDevice;
/// Prints the the Vulkan Device and API version
pub fn print_info(physical_device: &PhysicalDevice) {
    tracing::info!("API Version: {}", physical_device.api_version());
    tracing::info!(
        "Device Name:{} and Type:{:?}",
        physical_device.properties().device_name,
        physical_device.properties().device_type
    );
}
