use crate::{adapters::http, infra::config::Settings, state::AppState};
use anyhow::Result;
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tracing::info;

pub async fn run() -> Result<()> {
    social_core::env::load_dotenv();

    let settings = Settings::from_env()?;

    social_core::logging::init();

    let root = tracing::info_span!("app", service = "social-api", request_id = "-");
    let _enter = root.enter();

    let state = AppState::try_new(settings.clone()).await?;

    let app = http::router(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], settings.http_port));
    info!(%addr, "starting social-api");

    let listener = TcpListener::bind(addr).await?;
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

    Ok(())
}
