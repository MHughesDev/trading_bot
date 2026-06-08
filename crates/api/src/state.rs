use sqlx::PgPool;

/// Shared application state injected into every Axum handler.
#[derive(Clone)]
pub struct AppState {
    pub pg: PgPool,
}

impl AppState {
    pub fn new(pg: PgPool) -> Self {
        Self { pg }
    }
}
