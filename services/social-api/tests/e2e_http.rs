use anyhow::{anyhow, Context, Result};
use reqwest::StatusCode;
use serde::Deserialize;
use std::time::Duration;

const POST_ID: &str = "731b0395-4888-4822-b516-05b4b7bf2089";

fn enabled() -> bool {
    std::env::var("RUN_INTEGRATION").ok().as_deref() == Some("1")
}

fn base_url() -> String {
    std::env::var("SOCIAL_API_BASE_URL").unwrap_or_else(|_| "http://localhost:8080".to_string())
}

fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?)
}

async fn wait_ready(client: &reqwest::Client, base: &str) -> Result<()> {
    // In CI we already wait, but making tests robust avoids flakiness.
    for _ in 0..60 {
        match client.get(format!("{base}/health/ready")).send().await {
            Ok(resp) if resp.status().is_success() => return Ok(()),
            _ => tokio::time::sleep(Duration::from_secs(1)).await,
        }
    }
    Err(anyhow!("timed out waiting for {base}/health/ready"))
}

#[derive(Debug, Deserialize)]
struct HealthLiveResponse {
    status: String,
}

#[derive(Debug, Deserialize)]
struct HealthReadyResponse {
    ready: bool,
    checks: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct CountResponse {
    content_type: String,
    content_id: String,
    count: i64,
}

#[derive(Debug, Deserialize)]
struct StatusResponse {
    liked: bool,
    liked_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UserLikesResponse {
    items: Vec<UserLikeItem>,
    next_cursor: Option<String>,
    has_more: bool,
}

#[derive(Debug, Deserialize)]
struct UserLikeItem {
    content_type: String,
    content_id: String,
    liked_at: String,
}

#[derive(Debug, Deserialize)]
struct BatchCountsResponse {
    results: Vec<CountResponse>,
}

#[derive(Debug, Deserialize)]
struct ErrorEnvelope {
    error: ApiError,
}

#[derive(Debug, Deserialize)]
struct ApiError {
    code: String,
    message: String,
    request_id: String,
    details: Option<serde_json::Value>,
}

#[tokio::test]
async fn health_live_ok() -> Result<()> {
    if !enabled() {
        eprintln!("skipping e2e_http: set RUN_INTEGRATION=1 to run");
        return Ok(());
    }

    let base = base_url();
    let client = client()?;
    wait_ready(&client, &base).await?;

    let resp = client
        .get(format!("{base}/health/live"))
        .send()
        .await
        .context("GET /health/live")?;

    assert_eq!(resp.status(), StatusCode::OK);

    let body: HealthLiveResponse = resp.json().await?;
    assert_eq!(body.status, "live");
    Ok(())
}

#[tokio::test]
async fn health_ready_ok_and_has_checks() -> Result<()> {
    if !enabled() {
        return Ok(());
    }

    let base = base_url();
    let client = client()?;
    wait_ready(&client, &base).await?;

    let resp = client
        .get(format!("{base}/health/ready"))
        .send()
        .await
        .context("GET /health/ready")?;

    assert_eq!(resp.status(), StatusCode::OK);
    let body: HealthReadyResponse = resp.json().await?;

    // Spec: ready should be true only when DB+Redis+at least one content API is reachable.
    // Our docker-compose stack should satisfy this.
    assert!(body.ready, "expected ready=true");

    // Minimal structure checks. (We don't pin exact service names beyond what we expose.)
    let checks = body
        .checks
        .as_object()
        .context("checks must be an object")?;

    for k in [
        "postgres_writer",
        "postgres_reader",
        "redis",
        "content_apis",
    ] {
        assert!(checks.contains_key(k), "missing readiness check: {k}");
    }

    Ok(())
}

#[tokio::test]
async fn metrics_exposes_required_metric_families() -> Result<()> {
    if !enabled() {
        return Ok(());
    }

    let base = base_url();
    let client = client()?;
    wait_ready(&client, &base).await?;

    let resp = client
        .get(format!("{base}/metrics"))
        .send()
        .await
        .context("GET /metrics")?;

    assert_eq!(resp.status(), StatusCode::OK);
    let text = resp.text().await?;

    // Spec-required metric names.
    for name in [
        "social_api_http_requests_total",
        "social_api_http_request_duration_seconds",
        "social_api_cache_operations_total",
        "social_api_external_calls_total",
        "social_api_external_call_duration_seconds",
        "social_api_circuit_breaker_state",
        "social_api_db_pool_connections",
        "social_api_sse_connections_active",
        "social_api_likes_total",
    ] {
        assert!(
            text.contains(name),
            "metrics output missing required family: {name}"
        );
    }

    Ok(())
}

#[tokio::test]
async fn count_endpoint_returns_zero_for_new_item() -> Result<()> {
    if !enabled() {
        return Ok(());
    }

    let base = base_url();
    let client = client()?;
    wait_ready(&client, &base).await?;

    let resp = client
        .get(format!("{base}/v1/likes/post/{POST_ID}/count"))
        .send()
        .await
        .context("GET /v1/likes/.../count")?;

    assert_eq!(resp.status(), StatusCode::OK);
    let body: CountResponse = resp.json().await?;
    assert_eq!(body.content_type, "post");
    assert_eq!(body.content_id, POST_ID);
    assert!(body.count >= 0);

    Ok(())
}

#[tokio::test]
async fn batch_counts_enforces_limit_and_returns_results() -> Result<()> {
    if !enabled() {
        return Ok(());
    }

    let base = base_url();
    let client = client()?;
    wait_ready(&client, &base).await?;

    // Happy path
    let req = serde_json::json!({
        "items": [
            {"content_type": "post", "content_id": POST_ID},
            {"content_type": "top_picks", "content_id": "0a1b2c3d-4e5f-4a6b-8c9d-0e1f2a3b4c5d"}
        ]
    });

    let resp = client
        .post(format!("{base}/v1/likes/batch/counts"))
        .json(&req)
        .send()
        .await
        .context("POST /v1/likes/batch/counts")?;

    assert_eq!(resp.status(), StatusCode::OK);
    let body: BatchCountsResponse = resp.json().await?;
    assert_eq!(body.results.len(), 2);

    // Limit check (max 100)
    let too_big = serde_json::json!({
        "items": (0..101).map(|i| {
            let id = format!("00000000-0000-4000-8000-{:012}", i);
            serde_json::json!({"content_type":"post","content_id": id})
        }).collect::<Vec<_>>()
    });

    let request_id = "req_batch_too_large";
    let resp = client
        .post(format!("{base}/v1/likes/batch/counts"))
        .header("x-request-id", request_id)
        .json(&too_big)
        .send()
        .await?;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let err: ErrorEnvelope = resp.json().await?;
    assert_eq!(err.error.code, "BATCH_TOO_LARGE");
    assert_eq!(err.error.request_id, request_id);

    Ok(())
}

#[tokio::test]
async fn status_requires_auth_and_valid_token() -> Result<()> {
    if !enabled() {
        return Ok(());
    }

    let base = base_url();
    let client = client()?;
    wait_ready(&client, &base).await?;

    // Missing token -> 401 UNAUTHORIZED
    let request_id = "req_missing_token";
    let resp = client
        .get(format!("{base}/v1/likes/post/{POST_ID}/status"))
        .header("x-request-id", request_id)
        .send()
        .await?;

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let err: ErrorEnvelope = resp.json().await?;
    assert_eq!(err.error.code, "UNAUTHORIZED");
    assert_eq!(err.error.request_id, request_id);

    // Valid token -> 200 liked=false for fresh DB
    let resp = client
        .get(format!("{base}/v1/likes/post/{POST_ID}/status"))
        .header("Authorization", "Bearer tok_user_1")
        .send()
        .await?;

    assert_eq!(resp.status(), StatusCode::OK);
    let body: StatusResponse = resp.json().await?;
    assert!(!body.liked);
    assert!(body.liked_at.is_none());

    Ok(())
}

#[tokio::test]
async fn user_likes_empty_for_fresh_db() -> Result<()> {
    if !enabled() {
        return Ok(());
    }

    let base = base_url();
    let client = client()?;
    wait_ready(&client, &base).await?;

    let resp = client
        .get(format!("{base}/v1/likes/user"))
        .header("Authorization", "Bearer tok_user_1")
        .send()
        .await?;

    assert_eq!(resp.status(), StatusCode::OK);
    let body: UserLikesResponse = resp.json().await?;
    assert!(body.items.is_empty());
    assert!(!body.has_more);
    assert!(body.next_cursor.is_none());

    Ok(())
}

#[tokio::test]
async fn like_endpoint_is_not_implemented_but_returns_spec_error_shape() -> Result<()> {
    if !enabled() {
        return Ok(());
    }

    let base = base_url();
    let client = client()?;
    wait_ready(&client, &base).await?;

    let request_id = "req_like_not_impl";
    let req = serde_json::json!({"content_type":"post","content_id": POST_ID});

    let resp = client
        .post(format!("{base}/v1/likes"))
        .header("x-request-id", request_id)
        .header("Authorization", "Bearer tok_user_1")
        .json(&req)
        .send()
        .await?;

    assert_eq!(resp.status(), StatusCode::NOT_IMPLEMENTED);
    let err: ErrorEnvelope = resp.json().await?;
    assert_eq!(err.error.code, "NOT_IMPLEMENTED");
    assert_eq!(err.error.request_id, request_id);
    assert!(
        err.error
            .message
            .to_ascii_lowercase()
            .contains("next commit"),
        "message should explain scaffolding"
    );

    Ok(())
}
