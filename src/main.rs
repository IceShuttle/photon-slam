use winit::event_loop::{self, EventLoop};
mod window;

fn main() {
    let event_loop = EventLoop::new().unwrap();
    let mut app = window::App::new(&event_loop);
    event_loop.set_control_flow(event_loop::ControlFlow::Poll);
    event_loop.run_app(&mut app).unwrap();
}
