use photon_slam::utils::tracing::setup_tracing;
use winit::event_loop::{self, EventLoop};

fn main() {
    setup_tracing();
    let event_loop = EventLoop::new().unwrap();
    let mut app = photon_slam::window::App::new(&event_loop).unwrap();
    event_loop.set_control_flow(event_loop::ControlFlow::Poll);
    event_loop.run_app(&mut app).unwrap();
}
