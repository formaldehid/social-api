use crate::{config::Settings, routes, state::AppState};
use anyhow::Result;
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tracing::info;

pub async fn run() -> Result<()> {
    social_core::env::load_dotenv();

    let settings = Settings::from_env()?;

    social_core::logging::init();

    let root = tracing::info_span!(
        "app",
        service = "mock-content-api",
        content_type = %settings.content_type,
        request_id = "-"
    );
    let _enter = root.enter();

    let state = AppState::new(settings.content_type.clone());

    let app = routes::router(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], settings.http_port));
    info!(%addr, content_type = %settings.content_type, "starting mock-content-api");

    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
