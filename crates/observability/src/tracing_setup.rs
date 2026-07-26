use tracing_subscriber::{fmt, prelude::*, EnvFilter};

pub fn init_tracing(service_name: &str, json_logs: bool) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    // Use set_global_default instead of .init() so we don't also call
    // LogTracer::init(), which would claim the `log::Log` global slot.
    // The nautilus-backtest engine calls log::set_boxed_logger on startup
    // and panics if something else already owns that slot.  Our platform
    // code uses tracing::* macros exclusively, so the log bridge is not
    // needed for correctness.
    if json_logs {
        let subscriber = tracing_subscriber::registry()
            .with(filter)
            .with(fmt::layer().json());
        tracing::subscriber::set_global_default(subscriber)
            .expect("failed to set global tracing subscriber");
    } else {
        let subscriber = tracing_subscriber::registry()
            .with(filter)
            .with(fmt::layer());
        tracing::subscriber::set_global_default(subscriber)
            .expect("failed to set global tracing subscriber");
    }

    tracing::info!(service = service_name, "tracing initialized");
}
