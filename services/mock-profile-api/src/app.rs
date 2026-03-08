use crate::{config::Settings, routes, state::AppState};
use anyhow::Result;
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tracing::info;

pub async fn run() -> Result<()> {
    social_core::env::load_dotenv();

    let settings = Settings::from_env()?;

    social_core::logging::init();

    let root = tracing::info_span!("app", service = "mock-profile-api", request_id = "-");
    let _enter = root.enter();

    let state = AppState::new();

    let app = routes::router(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], settings.http_port));
    info!(%addr, "starting mock-profile-api");

    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
