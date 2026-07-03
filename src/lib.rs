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

// #[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
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
