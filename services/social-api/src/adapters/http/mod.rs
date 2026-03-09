pub mod error;
pub mod handlers;
pub mod types;

use crate::{middleware, state::AppState};
use axum::{
    routing::{delete, get, post},
    Router,
};
use tower_http::{
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    trace::TraceLayer,
};

pub fn router(state: AppState) -> Router {
    let request_id_header = axum::http::HeaderName::from_static("x-request-id");
    let metrics = state.metrics.clone();
    let rate_limit_state = state.clone();

    let routes: Router = Router::new()
        .route("/health/live", get(handlers::health_live))
        .route("/health/ready", get(handlers::health_ready))
        .route("/metrics", get(handlers::metrics))
        .route("/v1/likes", post(handlers::like))
        .route(
            "/v1/likes/{content_type}/{content_id}",
            delete(handlers::unlike),
        )
        .route(
            "/v1/likes/{content_type}/{content_id}/count",
            get(handlers::get_like_count),
        )
        .route(
            "/v1/likes/{content_type}/{content_id}/status",
            get(handlers::get_like_status),
        )
        .route("/v1/likes/user", get(handlers::get_user_likes))
        .route("/v1/likes/batch/counts", post(handlers::batch_like_counts))
        .route(
            "/v1/likes/batch/statuses",
            post(handlers::batch_like_statuses),
        )
        .route("/v1/likes/top", get(handlers::top_liked))
        .route("/v1/likes/stream", get(handlers::stream))
        .with_state(state)
        // Rate limiting (shared Redis state). Applied after matching so it can use `MatchedPath`.
        .route_layer(axum::middleware::from_fn_with_state(
            rate_limit_state,
            middleware::rate_limit::enforce,
        ))
        // Applied after matching, so it can use `MatchedPath` for low-cardinality metrics + logs.
        .route_layer(
            TraceLayer::new_for_http()
                .make_span_with(|req: &axum::http::Request<axum::body::Body>| {
                    let request_id = req
                        .headers()
                        .get("x-request-id")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("-");

                    let matched_path = req
                        .extensions()
                        .get::<axum::extract::MatchedPath>()
                        .map(|mp| mp.as_str())
                        .unwrap_or("<unmatched>");

                    tracing::info_span!(
                        "http_request",
                        service = "social-api",
                        request_id = %request_id,
                        method = %req.method(),
                        path = %matched_path,
                        user_id = tracing::field::Empty,
                    )
                })
                .on_response(
                    |res: &axum::http::Response<axum::body::Body>,
                     latency: std::time::Duration,
                     _span: &tracing::Span| {
                        tracing::info!(
                            status = res.status().as_u16(),
                            latency_ms = latency.as_millis(),
                            "request completed"
                        );
                    },
                ),
        )
        .route_layer(axum::middleware::from_fn_with_state(
            metrics,
            middleware::metrics::track,
        ));

    routes
        // Request-id: generate if missing and propagate back in the response.
        .layer(SetRequestIdLayer::new(
            request_id_header.clone(),
            MakeRequestUuid,
        ))
        .layer(PropagateRequestIdLayer::new(request_id_header))
}
