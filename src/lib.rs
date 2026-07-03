#[cfg(target_os = "android")]
use winit::platform::android::activity::AndroidApp;

// use crate::window::App;
use winit::{
    event_loop::{ControlFlow, EventLoop},
    platform::android::EventLoopBuilderExtAndroid,
};
mod window;
use window::App;

use vulkano::device::physical::PhysicalDevice;

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
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

    let mut app = App::new(&event_loop);
    event_loop.set_control_flow(ControlFlow::Poll);
    event_loop.run_app(&mut app).unwrap();
}

/// Prints the the Vulkan Device and API version
pub fn print_info(physical_device: &PhysicalDevice) {
    tracing::info!("API Version: {}", physical_device.api_version());
    tracing::info!(
        "Device Name:{} and Type:{:?}",
        physical_device.properties().device_name,
        physical_device.properties().device_type
    );
}
