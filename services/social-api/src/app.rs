use crate::{adapters::http, infra::config::Settings, state::AppState};
use anyhow::Result;
use social_core::domain::LeaderboardWindow;
use social_core::ports::{LeaderboardCache, LeaderboardRepository};
use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::TcpListener;
use tracing::info;

pub async fn run() -> Result<()> {
    social_core::env::load_dotenv();

    let settings = Settings::from_env()?;

    social_core::logging::init();

    let root = tracing::info_span!("app", service = "social-api", request_id = "-");
    let _enter = root.enter();

    let state = AppState::try_new(settings.clone()).await?;

    // Background leaderboard refresh: pre-computes top lists into Redis so the
    // public endpoint can remain a fast cache read.
    //
    // This matches the provided env var LEADERBOARD_REFRESH_INTERVAL_SECS.
    tokio::spawn(leaderboard_refresher(state.clone()));

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

async fn leaderboard_refresher(state: AppState) {
    let interval_secs = state.settings.leaderboard_refresh_interval_secs.max(1);
    let mut ticker = tokio::time::interval(Duration::from_secs(interval_secs));

    loop {
        ticker.tick().await;

        for window in [
            LeaderboardWindow::H24,
            LeaderboardWindow::D7,
            LeaderboardWindow::D30,
            LeaderboardWindow::All,
        ] {
            // Global leaderboard (all content types)
            refresh_one(&state, window, None).await;

            // Per-content-type leaderboards
            for ct in state.content_registry.keys() {
                refresh_one(&state, window, Some(ct.as_str())).await;
            }
        }
    }
}

async fn refresh_one(state: &AppState, window: LeaderboardWindow, content_type: Option<&str>) {
    match state
        .leaderboard_repo
        .top_liked(window, content_type, 50)
        .await
    {
        Ok(items) => {
            if let Err(e) = state
                .leaderboard_cache
                .set_top_liked(window, content_type, &items)
                .await
            {
                tracing::warn!(
                    service = "social-api",
                    error_type = "leaderboard_cache",
                    error_message = %e,
                    window = %window,
                    content_type = content_type.unwrap_or("all"),
                    "failed to refresh leaderboard cache"
                );
            }
        }
        Err(e) => {
            tracing::warn!(
                service = "social-api",
                error_type = "leaderboard_repo",
                error_message = %e,
                window = %window,
                content_type = content_type.unwrap_or("all"),
                "failed to refresh leaderboard from db"
            );
        }
    }
}
