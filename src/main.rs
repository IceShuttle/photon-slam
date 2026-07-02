use tracing_subscriber::FmtSubscriber;
use winit::event_loop::{self, EventLoop};
mod window;

fn main() {
    setup_tracing();
    let event_loop = EventLoop::new().unwrap();
    let mut app = window::App::new(&event_loop);
    event_loop.set_control_flow(event_loop::ControlFlow::Poll);
    event_loop.run_app(&mut app).unwrap();
}

fn setup_tracing() {
    const DEFAULT_LOGGING: &str = "photon_slam=info,warn";

    let rust_log = std::env::var("RUST_LOG")
        .ok()
        .and_then(|s| if s.is_empty() { None } else { Some(s) })
        .unwrap_or_else(|| DEFAULT_LOGGING.to_owned());

    tracing::subscriber::set_global_default(
        FmtSubscriber::builder().with_env_filter(rust_log).finish(),
    )
    .expect("tracing setup failed");
    tracing::debug!("Debug mode");
}
