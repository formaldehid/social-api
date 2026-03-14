use crate::{adapters::http::error::ApiError, state::AppState};
use axum::{
    extract::{MatchedPath, State},
    http::{HeaderMap, HeaderValue, Request},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::{
    net::{IpAddr, SocketAddr},
    time::{SystemTime, UNIX_EPOCH},
};
use tracing::warn;

/// Enforces Redis-backed rate limiting.
///
/// Spec requirements:
/// - 30 requests/min per user on write endpoints (like/unlike)
/// - 1000 requests/min per IP on public read endpoints
/// - Include X-RateLimit-* headers on all responses
/// - Include Retry-After on 429 responses
pub async fn enforce(
    State(state): State<AppState>,
    req: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let matched_path = req
        .extensions()
        .get::<MatchedPath>()
        .map(|mp| mp.as_str())
        .unwrap_or("<unmatched>");

    let request_id = req
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("-")
        .to_string();

    let is_write = is_write_endpoint(req.method().as_str(), matched_path);

    let (scope, id, limit) = if is_write {
        let limit = state.settings.rate_limit_write_per_minute;

        // "Per-user" is enforced per auth token (tokens map 1:1 to users in the provided spec).
        // Note: in a production system you might want to hash the token before using it as a Redis key.
        let user_key =
            social_core::http::bearer_token(req.headers()).unwrap_or_else(|| client_ip_key(&req));

        ("write", user_key, limit)
    } else {
        let limit = state.settings.rate_limit_read_per_minute;
        ("read", client_ip_key(&req), limit)
    };

    let decision = match state.rate_limiter.check(scope, &id, limit).await {
        Ok(d) => d,
        Err(e) => {
            // Fail-open on Redis errors so the service continues operating (degraded mode).
            warn!(
                service = "social-api",
                request_id = %request_id,
                error_type = "rate_limit",
                error_message = %e,
                "rate limiter unavailable; allowing request (fail-open)"
            );

            let mut resp = next.run(req).await;
            // Best-effort headers even in degraded mode.
            let (reset_epoch, remaining) = fallback_window_headers(limit);
            set_rate_limit_headers(&mut resp, limit, remaining, reset_epoch, None);
            return resp;
        }
    };

    if !decision.allowed {
        let mut resp = ApiError::rate_limited(request_id).into_response();
        set_rate_limit_headers(
            &mut resp,
            decision.limit,
            decision.remaining,
            decision.reset_epoch,
            Some(decision.retry_after_secs),
        );
        return resp;
    }

    let mut resp = next.run(req).await;
    set_rate_limit_headers(
        &mut resp,
        decision.limit,
        decision.remaining,
        decision.reset_epoch,
        None,
    );
    resp
}

fn is_write_endpoint(method: &str, matched_path: &str) -> bool {
    // Write endpoints defined in the spec:
    // POST   /v1/likes
    // DELETE /v1/likes/{content_type}/{content_id}
    (method == "POST" && matched_path == "/v1/likes")
        || (method == "DELETE" && matched_path == "/v1/likes/{content_type}/{content_id}")
}

fn client_ip_key(req: &Request<axum::body::Body>) -> String {
    // Prefer X-Forwarded-For when present (common behind reverse proxies).
    if let Some(ip) = forwarded_for(req.headers()) {
        return ip.to_string();
    }

    // Fall back to ConnectInfo if the server is configured with connect_info.
    if let Some(ip) = connect_info_ip(req) {
        return ip.to_string();
    }

    "0.0.0.0".to_string()
}

fn forwarded_for(headers: &HeaderMap) -> Option<IpAddr> {
    let raw = headers.get("x-forwarded-for")?.to_str().ok()?;
    // Format: client, proxy1, proxy2...
    let first = raw.split(',').next()?.trim();
    first.parse::<IpAddr>().ok()
}

fn connect_info_ip(req: &Request<axum::body::Body>) -> Option<IpAddr> {
    req.extensions()
        .get::<axum::extract::ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip())
}

fn set_rate_limit_headers(
    resp: &mut Response,
    limit: u64,
    remaining: u64,
    reset_epoch: u64,
    retry_after_secs: Option<u64>,
) {
    // Header names are case-insensitive; we use lower-case constants.
    resp.headers_mut().insert(
        "x-ratelimit-limit",
        HeaderValue::from_str(&limit.to_string()).unwrap_or_else(|_| HeaderValue::from_static("0")),
    );
    resp.headers_mut().insert(
        "x-ratelimit-remaining",
        HeaderValue::from_str(&remaining.to_string())
            .unwrap_or_else(|_| HeaderValue::from_static("0")),
    );
    resp.headers_mut().insert(
        "x-ratelimit-reset",
        HeaderValue::from_str(&reset_epoch.to_string())
            .unwrap_or_else(|_| HeaderValue::from_static("0")),
    );

    if let Some(secs) = retry_after_secs {
        resp.headers_mut().insert(
            "retry-after",
            HeaderValue::from_str(&secs.to_string())
                .unwrap_or_else(|_| HeaderValue::from_static("0")),
        );
    }
}

fn fallback_window_headers(limit: u64) -> (u64, u64) {
    // If Redis is down we can't know the true remaining. Provide a best-effort value so clients
    // still get the required headers.
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let window_start = now - (now % 60);
    let reset = window_start + 60;
    (reset, limit)
}
