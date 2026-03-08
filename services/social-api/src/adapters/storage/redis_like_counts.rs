use crate::infra::metrics::Metrics;
use deadpool_redis::Pool;
use redis::AsyncCommands;
use social_core::{
    domain::ContentKey,
    ports::{CacheError, LikeCountsCache},
};
use std::{sync::Arc, time::Duration};

#[derive(Clone)]
pub struct RedisLikeCountsCache {
    pool: Pool,
    ttl: Duration,
    metrics: Arc<Metrics>,
}

impl RedisLikeCountsCache {
    pub fn new(pool: Pool, ttl: Duration, metrics: Arc<Metrics>) -> Self {
        Self { pool, ttl, metrics }
    }

    fn key_for(content: &ContentKey) -> String {
        format!(
            "likes:count:{}:{}",
            content.content_type, content.content_id
        )
    }
}

#[async_trait::async_trait]
impl LikeCountsCache for RedisLikeCountsCache {
    async fn get_count(&self, key: &ContentKey) -> Result<Option<i64>, CacheError> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| CacheError::Unavailable(e.to_string()))?;

        let redis_key = Self::key_for(key);
        let val: Option<String> = conn.get(&redis_key).await.map_err(|e| {
            self.metrics.cache_error("get");
            CacheError::Unavailable(e.to_string())
        })?;

        match val {
            Some(s) => {
                self.metrics.cache_hit("get");
                Ok(s.parse::<i64>().ok())
            }
            None => {
                self.metrics.cache_miss("get");
                Ok(None)
            }
        }
    }

    async fn set_count(&self, key: &ContentKey, count: i64) -> Result<(), CacheError> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| CacheError::Unavailable(e.to_string()))?;

        let redis_key = Self::key_for(key);
        let ttl_secs: u64 = self.ttl.as_secs();
        let _: () = conn
            .set_ex(redis_key, count.to_string(), ttl_secs)
            .await
            .map_err(|e| {
                self.metrics.cache_error("set");
                CacheError::Unavailable(e.to_string())
            })?;
        Ok(())
    }

    async fn get_counts(&self, keys: &[ContentKey]) -> Result<Vec<Option<i64>>, CacheError> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| CacheError::Unavailable(e.to_string()))?;

        let redis_keys: Vec<String> = keys.iter().map(Self::key_for).collect();
        let vals: Vec<Option<String>> = redis::cmd("MGET")
            .arg(redis_keys)
            .query_async(&mut conn)
            .await
            .map_err(|e| {
                self.metrics.cache_error("mget");
                CacheError::Unavailable(e.to_string())
            })?;

        if vals.iter().any(|v| v.is_some()) {
            self.metrics.cache_hit("mget");
        }
        if vals.iter().any(|v| v.is_none()) {
            self.metrics.cache_miss("mget");
        }

        Ok(vals
            .into_iter()
            .map(|opt| opt.and_then(|s| s.parse::<i64>().ok()))
            .collect())
    }

    async fn set_counts(&self, items: &[(ContentKey, i64)]) -> Result<(), CacheError> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| CacheError::Unavailable(e.to_string()))?;

        let ttl_secs = self.ttl.as_secs() as usize;
        let mut pipe = redis::pipe();
        for (k, count) in items {
            pipe.cmd("SETEX")
                .arg(Self::key_for(k))
                .arg(ttl_secs)
                .arg(count.to_string())
                .ignore();
        }

        pipe.query_async::<String>(&mut conn).await.map_err(|e| {
            self.metrics.cache_error("setex");
            CacheError::Unavailable(e.to_string())
        })?;

        Ok(())
    }
}
