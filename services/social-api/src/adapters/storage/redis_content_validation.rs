use crate::infra::metrics::Metrics;
use deadpool_redis::Pool;
use redis::AsyncCommands;
use social_core::domain::ContentKey;
use std::{sync::Arc, time::Duration};

#[derive(Clone)]
pub struct RedisContentValidationCache {
    pool: Pool,
    ttl: Duration,
    metrics: Arc<Metrics>,
}

impl RedisContentValidationCache {
    pub fn new(pool: Pool, ttl: Duration, metrics: Arc<Metrics>) -> Self {
        Self { pool, ttl, metrics }
    }

    fn key_for(key: &ContentKey) -> String {
        format!("content:exists:{}:{}", key.content_type, key.content_id)
    }

    fn decode(raw: &str) -> Option<bool> {
        match raw {
            "1" | "true" | "TRUE" => Some(true),
            "0" | "false" | "FALSE" => Some(false),
            _ => None,
        }
    }

    pub async fn get(&self, key: &ContentKey) -> anyhow::Result<Option<bool>> {
        let mut conn = self.pool.get().await.map_err(|e| {
            self.metrics.cache_error("content_validation_get");
            anyhow::anyhow!("redis pool unavailable: {e}")
        })?;

        let redis_key = Self::key_for(key);
        let val: Option<String> = conn.get(redis_key).await.map_err(|e| {
            self.metrics.cache_error("content_validation_get");
            anyhow::anyhow!("redis get failed: {e}")
        })?;

        match val {
            Some(s) => match Self::decode(&s) {
                Some(v) => {
                    self.metrics.cache_hit("content_validation_get");
                    Ok(Some(v))
                }
                None => {
                    self.metrics.cache_miss("content_validation_get");
                    Ok(None)
                }
            },
            None => {
                self.metrics.cache_miss("content_validation_get");
                Ok(None)
            }
        }
    }

    pub async fn set(&self, key: &ContentKey, exists: bool) -> anyhow::Result<()> {
        let mut conn = self.pool.get().await.map_err(|e| {
            self.metrics.cache_error("content_validation_set");
            anyhow::anyhow!("redis pool unavailable: {e}")
        })?;

        let redis_key = Self::key_for(key);
        let ttl_secs: usize = self.ttl.as_secs() as usize;
        let value = if exists { "1" } else { "0" };

        let _: () = redis::cmd("SETEX")
            .arg(redis_key)
            .arg(ttl_secs)
            .arg(value)
            .query_async(&mut conn)
            .await
            .map_err(|e| {
                self.metrics.cache_error("content_validation_set");
                anyhow::anyhow!("redis setex failed: {e}")
            })?;

        self.metrics.cache_hit("content_validation_set");
        Ok(())
    }
}
