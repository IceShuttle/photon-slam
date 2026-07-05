use tracing_subscriber::FmtSubscriber;
use winit::event_loop::{self, EventLoop};

fn main() {
    setup_tracing();
    let event_loop = EventLoop::new().unwrap();
    let mut app = photon_slam::window::App::new(&event_loop).unwrap();
    event_loop.set_control_flow(event_loop::ControlFlow::Poll);
    event_loop.run_app(&mut app).unwrap();
}

fn setup_tracing() {
    const DEFAULT_LOGGING: &str = "photon_slam=info,warn";

    let rust_log = std::env::var("RUST_LOG")
        .ok()
        .and_then(|s| if s.is_empty() { None } else { Some(s) })
        .unwrap_or_else(|| DEFAULT_LOGGING.to_owned());

    match tracing::subscriber::set_global_default(
        FmtSubscriber::builder().with_env_filter(rust_log).finish(),
    ) {
        Ok(_) => tracing::debug!("Debug mode"),
        Err(_) => eprintln!("Tracing cannot  be initiated"),
    }
}
