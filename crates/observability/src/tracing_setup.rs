use tracing_subscriber::{fmt, prelude::*, EnvFilter};

pub fn init_tracing(service_name: &str, json_logs: bool) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    if json_logs {
        tracing_subscriber::registry()
            .with(filter)
            .with(fmt::layer().json())
            .init();
    } else {
        tracing_subscriber::registry()
            .with(filter)
            .with(fmt::layer())
            .init();
    }

    tracing::info!(service = service_name, "tracing initialized");
}
