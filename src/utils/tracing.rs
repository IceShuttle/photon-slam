use tracing_subscriber::FmtSubscriber;

pub fn setup_tracing() {
    const DEFAULT_LOGGING: &str = "photon_slam=info,warn";

    let rust_log = std::env::var("RUST_LOG").unwrap_or(DEFAULT_LOGGING.to_owned());

    match tracing::subscriber::set_global_default(
        FmtSubscriber::builder().with_env_filter(rust_log).finish(),
    ) {
        Ok(_) => tracing::debug!("Debug mode"),
        Err(_) => eprintln!("Tracing cannot  be initiated"),
    }
}

#[macro_export]
macro_rules! trace_return {
    ($e:expr) => {
        tracing::error!("{:?}", $e);
        return;
    };
}
