use crate::{adapters::http, infra::config::Settings, state::AppState};
use anyhow::Result;
use social_core::domain::LeaderboardWindow;
use social_core::ports::{LeaderboardCache, LeaderboardRepository};
use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
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
    let mut leaderboard_handle = tokio::spawn(leaderboard_refresher(
        state.clone(),
        state.shutdown.subscribe(),
    ));

    let app = http::router(state.clone());

    let addr = SocketAddr::from(([0, 0, 0, 0], settings.http_port));
    info!(%addr, "starting social-api");

    let listener = TcpListener::bind(addr).await?;

    // Axum/Hyper graceful shutdown is triggered via an oneshot.
    // We want to:
    // 1) stop accepting new connections
    // 2) drain in-flight requests with a bounded timeout
    // 3) close SSE connections with a final shutdown event
    // 4) close DB/Redis pools
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

    let server = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        let _ = shutdown_rx.await;
    });

    let mut server_handle = tokio::spawn(async move { server.await });

    // Wait for SIGTERM (or Ctrl+C locally).
    shutdown_signal().await;
    info!(service = "social-api", "shutdown signal received");

    // (1) Stop accepting new connections.
    let _ = shutdown_tx.send(());
    info!(service = "social-api", "stop accepting new connections");

    // (3) Close SSE connections + stop background tasks.
    state.shutdown.trigger();
    info!(
        service = "social-api",
        "broadcast shutdown to SSE + background tasks"
    );

    // (2) Drain in-flight requests with bounded timeout.
    let timeout = Duration::from_secs(state.settings.shutdown_timeout_secs.max(1));
    match tokio::time::timeout(timeout, &mut server_handle).await {
        Ok(join_res) => {
            // Server finished.
            join_res.map_err(|e| anyhow::anyhow!("server task join error: {e}"))??;
            info!(service = "social-api", "http server drained");
        }
        Err(_) => {
            tracing::warn!(
                service = "social-api",
                shutdown_timeout_secs = state.settings.shutdown_timeout_secs,
                "shutdown timeout reached; forcing close"
            );
            // Best effort: abort the server task so the process can terminate.
            // Existing connections will be dropped.
            server_handle.abort();
        }
    }

    // Ensure background tasks have observed shutdown (best effort).
    // If they don't finish quickly, abort them so we can close pools and exit.
    if tokio::time::timeout(Duration::from_secs(2), &mut leaderboard_handle)
        .await
        .is_err()
    {
        tracing::warn!(
            service = "social-api",
            "leaderboard refresher did not stop in time; aborting"
        );
        leaderboard_handle.abort();
    }

    // (4) Flush pending metrics (Prometheus is pull-based; rendering once is best-effort).
    if let Err(e) = state.metrics.render() {
        tracing::warn!(
            service = "social-api",
            error_type = "metrics_flush",
            error_message = %e,
            "failed to render metrics during shutdown"
        );
    }

    // (5) Close DB pools. (Redis pool is dropped with AppState.)
    state.db_writer.close().await;
    state.db_reader.close().await;
    drop(state.redis);

    info!(service = "social-api", "shutdown complete");

    Ok(())
}

async fn leaderboard_refresher(state: AppState, mut shutdown: tokio::sync::watch::Receiver<bool>) {
    let interval_secs = state.settings.leaderboard_refresh_interval_secs.max(1);
    let mut ticker = tokio::time::interval(Duration::from_secs(interval_secs));

    loop {
        tokio::select! {
            _ = ticker.tick() => {}
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    tracing::info!(service = "social-api", "leaderboard refresher stopping");
                    break;
                }
            }
        }

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

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let sigterm = async {
        use tokio::signal::unix::{signal, SignalKind};
        let mut stream = signal(SignalKind::terminate()).expect("install SIGTERM handler");
        let _ = stream.recv().await;
    };

    #[cfg(not(unix))]
    let sigterm = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = sigterm => {},
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
