use crate::infra::metrics::Metrics;
use axum::{
    extract::{MatchedPath, State},
    http::Request,
    middleware::Next,
    response::Response,
};
use std::{sync::Arc, time::Instant};

pub async fn track(
    State(metrics): State<Arc<Metrics>>,
    req: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let start = Instant::now();
    let method = req.method().as_str().to_string();
    let matched_path = req
        .extensions()
        .get::<MatchedPath>()
        .map(|mp| mp.as_str().to_string())
        .unwrap_or_else(|| "<unmatched>".to_string());

    let response = next.run(req).await;
    let status = response.status().as_u16().to_string();

    metrics
        .http_requests_total
        .with_label_values(&[&method, &matched_path, &status])
        .inc();

    let elapsed = start.elapsed().as_secs_f64();
    metrics
        .http_request_duration_seconds
        .with_label_values(&[&method, &matched_path])
        .observe(elapsed);

    response
}
