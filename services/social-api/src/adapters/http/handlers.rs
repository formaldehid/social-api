use super::{error::ApiError, types::*};
use crate::adapters::storage::redis_like_events::LikeStreamEvent;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use chrono::Utc;
use futures::stream;
use social_core::domain::{ContentKey, LeaderboardWindow};
use social_core::ports::AuthProvider;
use social_core::ports::ContentCatalog;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;
use uuid::{Uuid, Version};

pub async fn health_live() -> Json<HealthLiveResponse> {
    Json(HealthLiveResponse { status: "live" })
}

pub async fn health_ready(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let request_id = request_id(&headers);

    let mut checks = serde_json::Map::new();
    let mut ready = true;

    // Postgres (writer)
    let pg_writer_ok = sqlx::query("SELECT 1")
        .execute(&state.db_writer)
        .await
        .is_ok();
    checks.insert(
        "postgres_writer".to_string(),
        serde_json::json!({ "ok": pg_writer_ok }),
    );
    ready &= pg_writer_ok;

    // Postgres (read)
    let pg_reader_ok = sqlx::query("SELECT 1")
        .execute(&state.db_reader)
        .await
        .is_ok();
    checks.insert(
        "postgres_reader".to_string(),
        serde_json::json!({ "ok": pg_reader_ok }),
    );
    ready &= pg_reader_ok;

    // Redis PING
    let redis_ok = match state.redis.get().await {
        Ok(mut conn) => redis::cmd("PING")
            .query_async::<String>(&mut conn)
            .await
            .is_ok(),
        Err(_) => false,
    };
    checks.insert("redis".to_string(), serde_json::json!({ "ok": redis_ok }));
    ready &= redis_ok;

    // Content APIs: at least one reachable (200 or 404)
    let mut reachable_any = false;
    let mut per_service = serde_json::Map::new();

    for (content_type, base_url) in state.content_registry.iter() {
        // Probe a random UUID: 404 is a valid "service reachable" signal.
        let probe_id = Uuid::new_v4();
        let url = match base_url.join(&format!("/v1/{}/{probe_id}", content_type)) {
            Ok(u) => u,
            Err(_) => continue,
        };
        let ok = match state.http_client.get(url).send().await {
            Ok(resp) => matches!(resp.status(), StatusCode::OK | StatusCode::NOT_FOUND),
            Err(_) => false,
        };
        per_service.insert(content_type.clone(), serde_json::json!({ "ok": ok }));
        reachable_any |= ok;
    }

    checks.insert(
        "content_apis".to_string(),
        serde_json::Value::Object(per_service),
    );
    ready &= reachable_any;

    let body = HealthReadyResponse {
        ready,
        checks: serde_json::Value::Object(checks),
    };

    if ready {
        (StatusCode::OK, Json(body)).into_response()
    } else {
        tracing::warn!(service = "social-api", request_id = %request_id, "readiness check failed");
        (StatusCode::SERVICE_UNAVAILABLE, Json(body)).into_response()
    }
}

pub async fn metrics(State(state): State<AppState>) -> Response {
    update_db_pool_metrics(&state);

    match state.metrics.render() {
        Ok(text) => (
            StatusCode::OK,
            [("content-type", "text/plain; version=0.0.4")],
            text,
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [("content-type", "text/plain")],
            format!("failed to render metrics: {e}"),
        )
            .into_response(),
    }
}

pub async fn get_like_count(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((content_type, content_id)): Path<(String, String)>,
) -> Result<Json<CountResponse>, ApiError> {
    let request_id = request_id(&headers);
    ensure_content_type_known(&state, &content_type, request_id.clone())?;
    let content_id = parse_uuid_v4(&content_id, request_id.clone())?;

    let key = ContentKey {
        content_type: content_type.clone(),
        content_id,
    };

    let count = state.like_counts.get_count(&key).await.map_err(|e| {
        tracing::error!(
            service = "social-api",
            request_id = %request_id,
            error_type = "like_counts",
            error_message = %e,
            "failed to get like count"
        );
        ApiError::dependency_unavailable("like counts storage unavailable", request_id.clone())
    })?;

    Ok(Json(CountResponse {
        content_type,
        content_id,
        count,
    }))
}

pub async fn batch_like_counts(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<BatchItemsRequest>,
) -> Result<Json<BatchCountsResponse>, ApiError> {
    let request_id = request_id(&headers);

    if req.items.len() > 100 {
        return Err(ApiError::batch_too_large(100, request_id));
    }

    let mut keys = Vec::with_capacity(req.items.len());
    for item in &req.items {
        ensure_content_type_known(&state, &item.content_type, request_id.clone())?;
        let id = parse_uuid_v4(&item.content_id, request_id.clone())?;
        keys.push(ContentKey {
            content_type: item.content_type.clone(),
            content_id: id,
        });
    }

    let counts = state.like_counts.get_counts(&keys).await.map_err(|e| {
        tracing::error!(
            service = "social-api",
            request_id = %request_id,
            error_type = "like_counts_batch",
            error_message = %e,
            "failed to get batch like counts"
        );
        ApiError::dependency_unavailable("like counts storage unavailable", request_id.clone())
    })?;

    let results = keys
        .into_iter()
        .zip(counts)
        .map(|(k, c)| CountResponse {
            content_type: k.content_type,
            content_id: k.content_id,
            count: c,
        })
        .collect();

    Ok(Json(BatchCountsResponse { results }))
}

pub async fn get_like_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((content_type, content_id)): Path<(String, String)>,
) -> Result<Json<StatusResponse>, ApiError> {
    let request_id = request_id(&headers);
    ensure_content_type_known(&state, &content_type, request_id.clone())?;
    let content_id = parse_uuid_v4(&content_id, request_id.clone())?;
    let user = authenticate(&state, &headers, request_id.clone()).await?;

    // Fill request span user_id (for logs) once we have it.
    tracing::Span::current().record("user_id", user.user_id.as_str());

    let key = ContentKey {
        content_type,
        content_id,
    };

    let liked_at = state
        .likes_repo
        .get_status(&user.user_id, &key)
        .await
        .map_err(|e| {
            tracing::error!(
                service = "social-api",
                request_id = %request_id,
                error_type = "db",
                error_message = %e,
                "failed to get like status"
            );
            ApiError::dependency_unavailable("database unavailable", request_id.clone())
        })?;

    Ok(Json(StatusResponse {
        liked: liked_at.is_some(),
        liked_at,
    }))
}

pub async fn batch_like_statuses(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<BatchItemsRequest>,
) -> Result<Json<BatchStatusesResponse>, ApiError> {
    let request_id = request_id(&headers);
    if req.items.len() > 100 {
        return Err(ApiError::batch_too_large(100, request_id));
    }

    let user = authenticate(&state, &headers, request_id.clone()).await?;

    tracing::Span::current().record("user_id", user.user_id.as_str());

    let mut keys = Vec::with_capacity(req.items.len());
    for item in &req.items {
        ensure_content_type_known(&state, &item.content_type, request_id.clone())?;
        let id = parse_uuid_v4(&item.content_id, request_id.clone())?;
        keys.push(ContentKey {
            content_type: item.content_type.clone(),
            content_id: id,
        });
    }

    let statuses = state
        .likes_repo
        .get_statuses_batch(&user.user_id, &keys)
        .await
        .map_err(|e| {
            tracing::error!(
                service = "social-api",
                request_id = %request_id,
                error_type = "db",
                error_message = %e,
                "failed to get batch statuses"
            );
            ApiError::dependency_unavailable("database unavailable", request_id.clone())
        })?;

    let results = keys
        .into_iter()
        .map(|k| {
            let liked_at = statuses.get(&k).cloned();
            BatchStatusResult {
                content_type: k.content_type,
                content_id: k.content_id,
                liked: liked_at.is_some(),
                liked_at,
            }
        })
        .collect();

    Ok(Json(BatchStatusesResponse { results }))
}

pub async fn get_user_likes(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<UserLikesQuery>,
) -> Result<Json<UserLikesResponse>, ApiError> {
    let request_id = request_id(&headers);
    let user = authenticate(&state, &headers, request_id.clone()).await?;

    tracing::Span::current().record("user_id", user.user_id.as_str());

    if let Some(ref ct) = q.content_type {
        ensure_content_type_known(&state, ct, request_id.clone())?;
    }

    let cursor = match q.cursor {
        Some(c) => Some(
            crate::adapters::storage::pg_likes::Cursor::decode(&c)
                .map_err(|_| ApiError::invalid_cursor(request_id.clone()))?,
        ),
        None => None,
    };

    let limit = q.limit.unwrap_or(20);

    let (items, next_cursor, has_more) = state
        .likes_repo
        .list_user_likes(
            &user.user_id,
            q.content_type.as_deref(),
            cursor.as_ref(),
            limit,
        )
        .await
        .map_err(|e| {
            tracing::error!(
                service = "social-api",
                request_id = %request_id,
                error_type = "db",
                error_message = %e,
                "failed to list user likes"
            );
            ApiError::dependency_unavailable("database unavailable", request_id.clone())
        })?;

    let out_items = items
        .into_iter()
        .map(|i| UserLikeItemResponse {
            content_type: i.content_type,
            content_id: i.content_id,
            liked_at: i.liked_at,
        })
        .collect();

    Ok(Json(UserLikesResponse {
        items: out_items,
        next_cursor: next_cursor.map(|c| c.encode()),
        has_more,
    }))
}

pub async fn like(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<LikeRequest>,
) -> Result<Response, ApiError> {
    let request_id = request_id(&headers);

    ensure_content_type_known(&state, &req.content_type, request_id.clone())?;
    let content_id = parse_uuid_v4(&req.content_id, request_id.clone())?;

    let user = authenticate(&state, &headers, request_id.clone()).await?;
    tracing::Span::current().record("user_id", user.user_id.as_str());

    let key = ContentKey {
        content_type: req.content_type.clone(),
        content_id,
    };

    // Validate content exists (required by spec)
    match state.content_catalog.exists(&key).await {
        Ok(true) => {}
        Ok(false) => {
            return Err(ApiError::content_not_found(
                &req.content_type,
                &req.content_id,
                request_id,
            ));
        }
        Err(e) => {
            return Err(match e {
                social_core::ports::ContentError::UnknownContentType(ct) => {
                    ApiError::content_type_unknown(&ct, request_id)
                }
                social_core::ports::ContentError::DependencyUnavailable(msg) => {
                    ApiError::dependency_unavailable(msg, request_id)
                }
            })
        }
    }

    let res = state
        .likes_writer
        .like(&user.user_id, &key)
        .await
        .map_err(|e| {
            tracing::error!(
                service = "social-api",
                request_id = %request_id,
                error_type = "db",
                error_message = %e,
                "failed to like"
            );
            ApiError::dependency_unavailable("database unavailable", request_id.clone())
        })?;

    // Update cached count atomically (best-effort).
    if let Err(e) = state
        .like_counts_cache
        .set_count_cas(&key, res.count, res.seq)
        .await
    {
        tracing::warn!(
            service = "social-api",
            request_id = %request_id,
            error_type = "cache",
            error_message = %e,
            "failed to update like count cache"
        );
    }

    state
        .metrics
        .likes_total
        .with_label_values(&[&req.content_type, &"like".to_string()])
        .inc();

    // Publish SSE event only when the like actually changed state.
    if !res.already_existed {
        let ev = LikeStreamEvent::like(&user.user_id, res.count, res.liked_at);
        if let Err(e) = state.like_events.publish(&key, &ev).await {
            tracing::warn!(
                service = "social-api",
                request_id = %request_id,
                content_type = %key.content_type,
                content_id = %key.content_id,
                error_type = "sse_publish",
                error_message = %e,
                "failed to publish like event"
            );
        }
    }

    let body = LikeResponse {
        liked: true,
        already_existed: res.already_existed,
        count: res.count,
        liked_at: res.liked_at,
    };

    Ok((StatusCode::CREATED, Json(body)).into_response())
}

pub async fn unlike(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((content_type, content_id)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let request_id = request_id(&headers);

    ensure_content_type_known(&state, &content_type, request_id.clone())?;
    let id = parse_uuid_v4(&content_id, request_id.clone())?;

    let user = authenticate(&state, &headers, request_id.clone()).await?;
    tracing::Span::current().record("user_id", user.user_id.as_str());

    let key = ContentKey {
        content_type: content_type.clone(),
        content_id: id,
    };

    match state.content_catalog.exists(&key).await {
        Ok(true) => {}
        Ok(false) => {
            return Err(ApiError::content_not_found(
                &content_type,
                &content_id,
                request_id,
            ));
        }
        Err(e) => {
            return Err(match e {
                social_core::ports::ContentError::UnknownContentType(ct) => {
                    ApiError::content_type_unknown(&ct, request_id)
                }
                social_core::ports::ContentError::DependencyUnavailable(msg) => {
                    ApiError::dependency_unavailable(msg, request_id)
                }
            })
        }
    }

    let res = state
        .likes_writer
        .unlike(&user.user_id, &key)
        .await
        .map_err(|e| {
            tracing::error!(
                service = "social-api",
                request_id = %request_id,
                error_type = "db",
                error_message = %e,
                "failed to unlike"
            );
            ApiError::dependency_unavailable("database unavailable", request_id.clone())
        })?;

    if let Err(e) = state
        .like_counts_cache
        .set_count_cas(&key, res.count, res.seq)
        .await
    {
        tracing::warn!(
            service = "social-api",
            request_id = %request_id,
            error_type = "cache",
            error_message = %e,
            "failed to update like count cache"
        );
    }

    state
        .metrics
        .likes_total
        .with_label_values(&[&content_type, &"unlike".to_string()])
        .inc();

    // Publish SSE event only when an unlike actually removed a like.
    if res.was_liked {
        let ev = LikeStreamEvent::unlike(&user.user_id, res.count, Utc::now());
        if let Err(e) = state.like_events.publish(&key, &ev).await {
            tracing::warn!(
                service = "social-api",
                request_id = %request_id,
                content_type = %key.content_type,
                content_id = %key.content_id,
                error_type = "sse_publish",
                error_message = %e,
                "failed to publish unlike event"
            );
        }
    }

    let body = UnlikeResponse {
        liked: false,
        was_liked: res.was_liked,
        count: res.count,
    };

    Ok((StatusCode::OK, Json(body)).into_response())
}

pub async fn top_liked(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TopLikedQuery>,
) -> Result<Json<TopLikedResponse>, ApiError> {
    let request_id = request_id(&headers);

    if let Some(ref ct) = q.content_type {
        ensure_content_type_known(&state, ct, request_id.clone())?;
    }

    // Default window: 24h (sensible for a "trending" leaderboard).
    let window_raw = q.window.as_deref().unwrap_or("24h");
    let window = LeaderboardWindow::parse(window_raw)
        .ok_or_else(|| ApiError::invalid_window(window_raw, request_id.clone()))?;

    let limit = q.limit.unwrap_or(10).clamp(1, 50);

    let items = state
        .leaderboard
        .get_top_liked(window, q.content_type.as_deref(), limit)
        .await
        .map_err(|e| {
            tracing::error!(
                service = "social-api",
                request_id = %request_id,
                error_type = "leaderboard",
                error_message = %e,
                "failed to get leaderboard"
            );
            ApiError::dependency_unavailable("leaderboard storage unavailable", request_id.clone())
        })?;

    let items = items
        .into_iter()
        .map(|i| TopLikedItem {
            content_type: i.content_type,
            content_id: i.content_id,
            count: i.count,
        })
        .collect();

    Ok(Json(TopLikedResponse {
        window: window.as_str().to_string(),
        content_type: q.content_type,
        items,
    }))
}

pub async fn stream(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<StreamQuery>,
) -> Result<Response, ApiError> {
    let request_id = request_id(&headers);

    ensure_content_type_known(&state, &q.content_type, request_id.clone())?;
    let content_id = parse_uuid_v4(&q.content_id, request_id.clone())?;

    let key = ContentKey {
        content_type: q.content_type,
        content_id,
    };

    let client_ip = client_ip(&headers);
    tracing::info!(
        service = "social-api",
        request_id = %request_id,
        client_ip = %client_ip,
        content_type = %key.content_type,
        content_id = %key.content_id,
        "sse connected"
    );

    // Bounded buffer to protect the process from slow clients.
    // If the client can't keep up, we drop events rather than growing memory.
    const SSE_BUFFER: usize = 128;

    let (tx, rx) = tokio::sync::mpsc::channel::<String>(SSE_BUFFER);

    // Track active connections.
    state.metrics.sse_connections_active.inc();
    let guard = SseConnectionGuard::new(state.metrics.clone(), request_id.clone(), client_ip, &key);

    // Subscribe to Redis PubSub channel for this content item.
    let _subscriber_task = state
        .like_events
        .spawn_subscriber(key.clone(), tx.clone(), request_id.clone())
        .await
        .map_err(|e| {
            tracing::warn!(
                service = "social-api",
                request_id = %request_id,
                content_type = %key.content_type,
                content_id = %key.content_id,
                error_type = "sse_pubsub",
                error_message = %e,
                "failed to initialize redis pubsub subscriber"
            );
            ApiError::dependency_unavailable("sse stream unavailable", request_id.clone())
        })?;

    // Heartbeats (spec requires explicit heartbeat events, not just SSE comments).
    let hb_secs = state.settings.sse_heartbeat_interval_secs.max(1);
    spawn_heartbeat(tx.clone(), hb_secs, request_id.clone());

    let stream = stream::unfold((rx, guard), |(mut rx, guard)| async {
        match rx.recv().await {
            Some(payload) => {
                let ev = axum::response::sse::Event::default().data(payload);
                Some((Ok::<_, Infallible>(ev), (rx, guard)))
            }
            None => None,
        }
    });

    let sse = axum::response::sse::Sse::new(stream);
    let mut resp = sse.into_response();

    // Helpful for nginx (prevents proxy buffering).
    resp.headers_mut().insert(
        "x-accel-buffering",
        axum::http::HeaderValue::from_static("no"),
    );

    Ok(resp)
}

fn spawn_heartbeat(tx: tokio::sync::mpsc::Sender<String>, hb_secs: u64, request_id: String) {
    tokio::spawn(async move {
        // Send an immediate heartbeat so clients can confirm the stream is live.
        if let Ok(payload) = serde_json::to_string(&LikeStreamEvent::heartbeat(Utc::now())) {
            let _ = tx.try_send(payload);
        }

        let mut ticker = tokio::time::interval(Duration::from_secs(hb_secs));
        loop {
            ticker.tick().await;
            if tx.is_closed() {
                break;
            }

            let payload = match serde_json::to_string(&LikeStreamEvent::heartbeat(Utc::now())) {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!(
                        service = "social-api",
                        request_id = %request_id,
                        error_type = "sse_heartbeat",
                        error_message = %e,
                        "failed to serialize heartbeat"
                    );
                    continue;
                }
            };

            // Best-effort: if the client is slow and the buffer is full, drop the heartbeat.
            let _ = tx.try_send(payload);
        }
    });
}

struct SseConnectionGuard {
    metrics: Arc<crate::infra::metrics::Metrics>,
    request_id: String,
    client_ip: String,
    content_type: String,
    content_id: String,
}

impl SseConnectionGuard {
    fn new(
        metrics: Arc<crate::infra::metrics::Metrics>,
        request_id: String,
        client_ip: String,
        key: &ContentKey,
    ) -> Self {
        Self {
            metrics,
            request_id,
            client_ip,
            content_type: key.content_type.clone(),
            content_id: key.content_id.to_string(),
        }
    }
}

impl Drop for SseConnectionGuard {
    fn drop(&mut self) {
        self.metrics.sse_connections_active.dec();
        tracing::info!(
            service = "social-api",
            request_id = %self.request_id,
            client_ip = %self.client_ip,
            content_type = %self.content_type,
            content_id = %self.content_id,
            "sse disconnected"
        );
    }
}

fn client_ip(headers: &HeaderMap) -> String {
    // Return an owned String to avoid lifetime issues when picking substrings.
    let pick_first = |raw: &str| raw.split(',').next().unwrap_or(raw).trim().to_string();

    if let Some(v) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        return pick_first(v);
    }

    if let Some(v) = headers.get("x-real-ip").and_then(|v| v.to_str().ok()) {
        return pick_first(v);
    }

    "unknown".to_string()
}

async fn authenticate(
    state: &AppState,
    headers: &HeaderMap,
    request_id: String,
) -> Result<social_core::domain::UserIdentity, ApiError> {
    let token = social_core::http::bearer_token(headers)
        .ok_or_else(|| ApiError::unauthorized(request_id.clone()))?;

    state
        .auth
        .validate_token(&token)
        .await
        .map_err(|e| match e {
            social_core::ports::AuthError::Unauthorized => {
                ApiError::unauthorized(request_id.clone())
            }
            social_core::ports::AuthError::DependencyUnavailable(msg) => {
                ApiError::dependency_unavailable(msg, request_id.clone())
            }
        })
}

fn request_id(headers: &HeaderMap) -> String {
    headers
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("-")
        .to_string()
}

fn ensure_content_type_known(
    state: &AppState,
    content_type: &str,
    request_id: String,
) -> Result<(), ApiError> {
    if state.content_registry.contains_key(content_type) {
        Ok(())
    } else {
        Err(ApiError::content_type_unknown(content_type, request_id))
    }
}

fn parse_uuid_v4(content_id: &str, request_id: String) -> Result<Uuid, ApiError> {
    let uuid = Uuid::parse_str(content_id)
        .map_err(|_| ApiError::invalid_content_id(content_id, request_id.clone()))?;
    if uuid.get_version() != Some(Version::Random) {
        return Err(ApiError::invalid_content_id(content_id, request_id));
    }
    Ok(uuid)
}

fn update_db_pool_metrics(state: &AppState) {
    let max = state.settings.db_max_connections as i64;

    let writer_size = state.db_writer.size() as i64;
    let writer_idle = state.db_writer.num_idle() as i64;
    let writer_active = (writer_size - writer_idle).max(0);

    let reader_size = state.db_reader.size() as i64;
    let reader_idle = state.db_reader.num_idle() as i64;
    let reader_active = (reader_size - reader_idle).max(0);

    state
        .metrics
        .db_pool_connections
        .with_label_values(&["active"])
        .set(writer_active + reader_active);
    state
        .metrics
        .db_pool_connections
        .with_label_values(&["idle"])
        .set(writer_idle + reader_idle);
    state
        .metrics
        .db_pool_connections
        .with_label_values(&["max"])
        .set(max);
}
