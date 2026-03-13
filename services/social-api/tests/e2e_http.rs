use anyhow::{anyhow, Context, Result};
use reqwest::StatusCode;
use serde::Deserialize;
use std::time::Duration;

const POST_ID: &str = "731b0395-4888-4822-b516-05b4b7bf2089";
const POST_ID_2: &str = "9601c044-6130-4ee5-a155-96570e05a02f";
const POST_ID_SSE: &str = "933dde0f-4744-4a66-9a38-bf5cb1f67553";

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
struct LikeResponse {
    liked: bool,
    already_existed: bool,
    count: i64,
    liked_at: String,
}

#[derive(Debug, Deserialize)]
struct UnlikeResponse {
    liked: bool,
    was_liked: bool,
    count: i64,
}

#[derive(Debug, Deserialize)]
struct StreamEvent {
    event: String,
    user_id: Option<String>,
    count: Option<i64>,
    timestamp: String,
}

#[derive(Debug, Deserialize)]
struct TopLikedResponse {
    window: String,
    content_type: Option<String>,
    items: Vec<TopLikedItem>,
}

#[derive(Debug, Deserialize)]
struct TopLikedItem {
    content_type: String,
    content_id: String,
    count: i64,
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

    // Valid token -> 200 liked=false for a user that hasn't liked this item
    let resp = client
        .get(format!("{base}/v1/likes/post/{POST_ID}/status"))
        .header("Authorization", "Bearer tok_user_2")
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
        .header("Authorization", "Bearer tok_user_2")
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
async fn like_unlike_lifecycle_is_idempotent_and_updates_counts() -> Result<()> {
    if !enabled() {
        return Ok(());
    }

    let base = base_url();
    let client = client()?;
    wait_ready(&client, &base).await?;

    // Ensure the test user starts in an "unliked" state even across re-runs.
    let _ = client
        .delete(format!("{base}/v1/likes/post/{POST_ID}"))
        .header("Authorization", "Bearer tok_user_1")
        .send()
        .await?;

    // baseline count
    let before: CountResponse = client
        .get(format!("{base}/v1/likes/post/{POST_ID}/count"))
        .send()
        .await?
        .json()
        .await?;

    // Like 1 -> should increment count by 1 and return 201
    let like_req = serde_json::json!({"content_type":"post","content_id": POST_ID});
    let resp = client
        .post(format!("{base}/v1/likes"))
        .header("Authorization", "Bearer tok_user_1")
        .json(&like_req)
        .send()
        .await?;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let like1: LikeResponse = resp.json().await?;
    assert!(like1.liked);
    assert!(!like1.already_existed);
    assert_eq!(like1.count, before.count + 1);
    assert!(!like1.liked_at.is_empty());

    // Like 2 (idempotent) -> count unchanged, already_existed=true
    let resp = client
        .post(format!("{base}/v1/likes"))
        .header("Authorization", "Bearer tok_user_1")
        .json(&like_req)
        .send()
        .await?;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let like2: LikeResponse = resp.json().await?;
    assert!(like2.liked);
    assert!(like2.already_existed);
    assert_eq!(like2.count, like1.count);
    assert_eq!(like2.liked_at, like1.liked_at);

    // Status should now be liked
    let status: StatusResponse = client
        .get(format!("{base}/v1/likes/post/{POST_ID}/status"))
        .header("Authorization", "Bearer tok_user_1")
        .send()
        .await?
        .json()
        .await?;
    assert!(status.liked);
    assert!(status.liked_at.is_some());

    // Unlike 1 -> decrement count by 1
    let resp = client
        .delete(format!("{base}/v1/likes/post/{POST_ID}"))
        .header("Authorization", "Bearer tok_user_1")
        .send()
        .await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let un1: UnlikeResponse = resp.json().await?;
    assert!(!un1.liked);
    assert!(un1.was_liked);
    assert_eq!(un1.count, like1.count - 1);

    // Unlike 2 (idempotent) -> count unchanged, was_liked=false
    let resp = client
        .delete(format!("{base}/v1/likes/post/{POST_ID}"))
        .header("Authorization", "Bearer tok_user_1")
        .send()
        .await?;
    assert_eq!(resp.status(), StatusCode::OK);
    let un2: UnlikeResponse = resp.json().await?;
    assert!(!un2.liked);
    assert!(!un2.was_liked);
    assert_eq!(un2.count, un1.count);

    // Count endpoint should match
    let after: CountResponse = client
        .get(format!("{base}/v1/likes/post/{POST_ID}/count"))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(after.count, un2.count);

    Ok(())
}

#[tokio::test]
async fn rate_limit_headers_present_on_count() -> Result<()> {
    if !enabled() {
        return Ok(());
    }

    let base = base_url();
    let client = client()?;
    wait_ready(&client, &base).await?;

    let resp = client
        .get(format!("{base}/v1/likes/post/{POST_ID}/count"))
        .send()
        .await?;

    assert_eq!(resp.status(), StatusCode::OK);

    // Spec: X-RateLimit-* headers should be included on all responses.
    for h in [
        "x-ratelimit-limit",
        "x-ratelimit-remaining",
        "x-ratelimit-reset",
    ] {
        assert!(
            resp.headers().get(h).is_some(),
            "missing rate limit header: {h}"
        );
    }

    // Basic sanity: headers should be parseable integers
    let _limit: u64 = resp
        .headers()
        .get("x-ratelimit-limit")
        .unwrap()
        .to_str()?
        .parse()?;
    let _remaining: u64 = resp
        .headers()
        .get("x-ratelimit-remaining")
        .unwrap()
        .to_str()?
        .parse()?;
    let _reset: u64 = resp
        .headers()
        .get("x-ratelimit-reset")
        .unwrap()
        .to_str()?
        .parse()?;

    Ok(())
}

#[tokio::test]
async fn write_rate_limit_returns_429_and_retry_after() -> Result<()> {
    if !enabled() {
        return Ok(());
    }

    let base = base_url();
    let client = client()?;
    wait_ready(&client, &base).await?;

    // Ensure the test user starts in an "unliked" state even across re-runs.
    let _ = client
        .delete(format!("{base}/v1/likes/post/{POST_ID_2}"))
        .header("Authorization", "Bearer tok_user_5")
        .send()
        .await?;

    let like_req = serde_json::json!({"content_type":"post","content_id": POST_ID_2});

    // Default write limit is 30 req/min. We send more than that and expect a 429.
    let mut saw_429 = false;
    let mut retry_after_seen = false;

    for _ in 0..40 {
        let resp = client
            .post(format!("{base}/v1/likes"))
            .header("Authorization", "Bearer tok_user_5")
            .json(&like_req)
            .send()
            .await?;

        let status = resp.status();
        let headers = resp.headers().clone();

        if status == StatusCode::TOO_MANY_REQUESTS {
            saw_429 = true;

            // Verify error envelope + header requirements.
            let err: ErrorEnvelope = resp.json().await?;
            assert_eq!(err.error.code, "RATE_LIMITED");

            assert!(
                headers.get("retry-after").is_some(),
                "expected Retry-After on 429"
            );
            retry_after_seen = true;

            for h in [
                "x-ratelimit-limit",
                "x-ratelimit-remaining",
                "x-ratelimit-reset",
            ] {
                assert!(
                    headers.get(h).is_some(),
                    "missing rate limit header on 429: {h}"
                );
            }

            break;
        }
    }

    assert!(
        saw_429,
        "expected at least one 429 after exceeding the write rate limit"
    );
    assert!(retry_after_seen, "expected Retry-After header on 429");

    Ok(())
}

#[tokio::test]
async fn leaderboard_top_liked_returns_200_and_valid_shape() -> Result<()> {
    if !enabled() {
        return Ok(());
    }

    let base = base_url();
    let client = client()?;
    wait_ready(&client, &base).await?;

    let resp = client
        .get(format!("{base}/v1/likes/top?window=7d&limit=10"))
        .send()
        .await
        .context("GET /v1/likes/top")?;

    assert_eq!(resp.status(), StatusCode::OK);

    let body: TopLikedResponse = resp.json().await?;
    assert_eq!(body.window, "7d");
    assert!(body.items.len() <= 10);

    Ok(())
}

#[tokio::test]
async fn leaderboard_rejects_invalid_window() -> Result<()> {
    if !enabled() {
        return Ok(());
    }

    let base = base_url();
    let client = client()?;
    wait_ready(&client, &base).await?;

    let request_id = "req_invalid_window";
    let resp = client
        .get(format!("{base}/v1/likes/top?window=bad"))
        .header("x-request-id", request_id)
        .send()
        .await?;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let err: ErrorEnvelope = resp.json().await?;
    assert_eq!(err.error.code, "INVALID_WINDOW");
    assert_eq!(err.error.request_id, request_id);

    Ok(())
}

async fn next_sse_data(resp: &mut reqwest::Response, buf: &mut String) -> Result<String> {
    loop {
        if let Some(pos) = buf.find('\n') {
            let mut line = buf[..pos].to_string();
            *buf = buf[pos + 1..].to_string();

            // Trim CRLF.
            if line.ends_with('\r') {
                line.pop();
            }

            if let Some(rest) = line.strip_prefix("data: ") {
                return Ok(rest.to_string());
            }

            continue;
        }

        let chunk = resp
            .chunk()
            .await
            .context("reading sse chunk")?
            .ok_or_else(|| anyhow!("sse stream ended"))?;
        buf.push_str(&String::from_utf8_lossy(chunk.as_ref()));
    }
}

async fn next_sse_data_with_timeout(
    resp: &mut reqwest::Response,
    buf: &mut String,
    timeout: Duration,
) -> Result<String> {
    match tokio::time::timeout(timeout, next_sse_data(resp, buf)).await {
        Ok(r) => r,
        Err(_) => Err(anyhow!("timed out waiting for sse data")),
    }
}

#[tokio::test]
async fn sse_stream_receives_like_and_unlike_events() -> Result<()> {
    if !enabled() {
        return Ok(());
    }

    let base = base_url();
    let client = client()?;
    wait_ready(&client, &base).await?;

    // Ensure the test user starts in an "unliked" state even across re-runs.
    let _ = client
        .delete(format!("{base}/v1/likes/post/{POST_ID_SSE}"))
        .header("Authorization", "Bearer tok_user_3")
        .send()
        .await?;

    let sse_client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .build()?;

    let mut sse_resp = sse_client
        .get(format!(
            "{base}/v1/likes/stream?content_type=post&content_id={POST_ID_SSE}"
        ))
        .send()
        .await
        .context("GET /v1/likes/stream")?;

    assert_eq!(sse_resp.status(), StatusCode::OK);

    let mut buf = String::new();

    let first = next_sse_data(&mut sse_resp, &mut buf).await?;
    let ev: StreamEvent = serde_json::from_str(&first)?;

    assert_eq!(ev.event, "heartbeat");
    assert!(!ev.timestamp.is_empty());

    // Small delay to reduce the chance of racing Redis subscribe setup.
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Trigger LIKE.
    let like_req = serde_json::json!({"content_type":"post","content_id": POST_ID_SSE});
    let like_resp = client
        .post(format!("{base}/v1/likes"))
        .header("Authorization", "Bearer tok_user_3")
        .json(&like_req)
        .send()
        .await?;

    assert_eq!(like_resp.status(), StatusCode::CREATED);

    // Read until we see the like event, but keep the wait bounded.
    let like_deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let like_ev = loop {
        let now = tokio::time::Instant::now();
        if now >= like_deadline {
            return Err(anyhow!("timed out waiting for like event"));
        }

        let raw = next_sse_data_with_timeout(
            &mut sse_resp,
            &mut buf,
            (like_deadline - now).min(Duration::from_secs(5)),
        )
        .await?;

        let ev: StreamEvent = serde_json::from_str(&raw)?;
        if ev.event == "like" {
            break ev;
        }
    };

    assert_eq!(
        like_ev.user_id.as_deref(),
        Some("usr_550e8400-e29b-41d4-a716-446655440003")
    );
    assert!(like_ev.count.unwrap_or(0) >= 1);

    // Trigger UNLIKE.
    let unlike_resp = client
        .delete(format!("{base}/v1/likes/post/{POST_ID_SSE}"))
        .header("Authorization", "Bearer tok_user_3")
        .send()
        .await?;
    assert_eq!(unlike_resp.status(), StatusCode::OK);

    let unlike_deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let unlike_ev = loop {
        let now = tokio::time::Instant::now();
        if now >= unlike_deadline {
            return Err(anyhow!("timed out waiting for unlike event"));
        }

        let raw = next_sse_data_with_timeout(
            &mut sse_resp,
            &mut buf,
            (unlike_deadline - now).min(Duration::from_secs(5)),
        )
        .await?;

        let ev: StreamEvent = serde_json::from_str(&raw)?;
        if ev.event == "unlike" {
            break ev;
        }
    };

    assert_eq!(
        unlike_ev.user_id.as_deref(),
        Some("usr_550e8400-e29b-41d4-a716-446655440003")
    );
    assert!(unlike_ev.count.unwrap_or(0) >= 0);

    Ok(())
}
