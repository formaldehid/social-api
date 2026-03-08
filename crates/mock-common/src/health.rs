use axum::{routing::get, Json, Router};
use serde::Serialize;

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
}

async fn health_live() -> Json<HealthResponse> {
    Json(HealthResponse { status: "live" })
}

async fn health_ready() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ready" })
}

/// Adds `/health/live` + `/health/ready` to an existing router.
/// Works for any router state type.
pub fn add_routes<S>(router: Router<S>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    router
        .route("/health/live", get(health_live))
        .route("/health/ready", get(health_ready))
}
