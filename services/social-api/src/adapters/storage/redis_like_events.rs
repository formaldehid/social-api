use crate::infra::metrics::Metrics;
use chrono::{DateTime, Utc};
use deadpool_redis::Pool;
use futures::StreamExt;
use redis::AsyncCommands;
use serde::Serialize;
use social_core::domain::ContentKey;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Payload pushed over the SSE stream.
///
/// Spec examples:
/// {"event":"like","user_id":"usr_...","count":42,"timestamp":"..."}
/// {"event":"unlike","user_id":"usr_...","count":41,"timestamp":"..."}
/// {"event":"heartbeat","timestamp":"..."}
#[derive(Debug, Clone, Serialize)]
pub struct LikeStreamEvent {
    pub event: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<i64>,
    pub timestamp: DateTime<Utc>,
}

impl LikeStreamEvent {
    pub fn like(user_id: &str, count: i64, timestamp: DateTime<Utc>) -> Self {
        Self {
            event: "like".to_string(),
            user_id: Some(user_id.to_string()),
            count: Some(count),
            timestamp,
        }
    }

    pub fn unlike(user_id: &str, count: i64, timestamp: DateTime<Utc>) -> Self {
        Self {
            event: "unlike".to_string(),
            user_id: Some(user_id.to_string()),
            count: Some(count),
            timestamp,
        }
    }

    pub fn heartbeat(timestamp: DateTime<Utc>) -> Self {
        Self {
            event: "heartbeat".to_string(),
            user_id: None,
            count: None,
            timestamp,
        }
    }

    /// Used during graceful shutdown: send one final event before closing.
    ///
    /// NOTE: full SIGTERM graceful shutdown is implemented separately, but we keep
    /// the event format here so the SSE handler can easily support it.
    pub fn shutdown(timestamp: DateTime<Utc>) -> Self {
        Self {
            event: "shutdown".to_string(),
            user_id: None,
            count: None,
            timestamp,
        }
    }
}

/// Redis-backed event bus for Like events.
///
/// Publishing uses Redis `PUBLISH`. Subscribing uses Redis PubSub on a dedicated
/// connection per SSE client.
#[derive(Clone)]
pub struct RedisLikeEventBus {
    pool: Pool,
    redis_url: String,
}

impl RedisLikeEventBus {
    pub fn new(pool: Pool, redis_url: String, _metrics: Arc<Metrics>) -> Self {
        Self { pool, redis_url }
    }

    pub fn channel_for(key: &ContentKey) -> String {
        format!("likes:events:{}:{}", key.content_type, key.content_id)
    }

    /// Best-effort publish (write path). Failures must not break the API request.
    pub async fn publish(&self, key: &ContentKey, event: &LikeStreamEvent) -> anyhow::Result<()> {
        let payload = serde_json::to_string(event)?;

        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| anyhow::anyhow!("redis pool unavailable for publish: {e}"))?;

        let channel = Self::channel_for(key);
        let _: i64 = conn
            .publish(channel, payload)
            .await
            .map_err(|e| anyhow::anyhow!("redis publish failed: {e}"))?;

        Ok(())
    }

    /// Spawns a subscriber task that forwards PubSub messages into the provided channel.
    ///
    /// Backpressure strategy:
    /// - Uses `try_send` into a bounded mpsc channel.
    /// - If the channel is full (slow client), we drop the message and log at WARN.
    pub async fn spawn_subscriber(
        &self,
        key: ContentKey,
        tx: mpsc::Sender<String>,
        request_id: String,
    ) -> anyhow::Result<tokio::task::JoinHandle<()>> {
        let redis_url = self.redis_url.clone();
        let channel = Self::channel_for(&key);

        // Establish and subscribe BEFORE returning, so callers know the stream is ready.
        let client = redis::Client::open(redis_url)
            .map_err(|e| anyhow::anyhow!("failed to create redis client for pubsub: {e}"))?;

        let mut pubsub = client
            .get_async_pubsub()
            .await
            .map_err(|e| anyhow::anyhow!("failed to create redis async pubsub connection: {e}"))?;

        pubsub.subscribe(&channel).await.map_err(|e| {
            anyhow::anyhow!("failed to subscribe to redis pubsub channel {channel}: {e}")
        })?;

        Ok(tokio::spawn(async move {
            let mut stream = pubsub.on_message();

            loop {
                tokio::select! {
                    _ = tx.closed() => {
                        // The HTTP client disconnected.
                        break;
                    }
                    msg = stream.next() => {
                        let msg = match msg {
                            Some(m) => m,
                            None => break,
                        };

                        let payload: String = match msg.get_payload() {
                            Ok(p) => p,
                            Err(e) => {
                                tracing::warn!(
                                    service = "social-api",
                                    request_id = %request_id,
                                    error_type = "sse_pubsub",
                                    error_message = %e,
                                    "failed to decode pubsub payload"
                                );
                                continue;
                            }
                        };

                        match tx.try_send(payload) {
                            Ok(()) => {}
                            Err(mpsc::error::TrySendError::Full(_)) => {
                                tracing::warn!(
                                    service = "social-api",
                                    request_id = %request_id,
                                    error_type = "sse_backpressure",
                                    channel = %channel,
                                    "dropping sse event due to slow client"
                                );
                            }
                            Err(mpsc::error::TrySendError::Closed(_)) => break,
                        }
                    }
                }
            }
        }))
    }
}
