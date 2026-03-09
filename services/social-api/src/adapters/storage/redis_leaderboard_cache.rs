use crate::infra::metrics::Metrics;
use deadpool_redis::Pool;
use redis::AsyncCommands;
use social_core::{
    domain::{LeaderboardWindow, LikeCount},
    ports::{CacheError, LeaderboardCache},
};
use std::{sync::Arc, time::Duration};

#[derive(Clone)]
pub struct RedisLeaderboardCache {
    pool: Pool,
    ttl: Duration,
    metrics: Arc<Metrics>,
}

impl RedisLeaderboardCache {
    pub fn new(pool: Pool, ttl: Duration, metrics: Arc<Metrics>) -> Self {
        Self { pool, ttl, metrics }
    }

    fn key_for(window: LeaderboardWindow, content_type: Option<&str>) -> String {
        let ct = content_type.unwrap_or("all");
        format!("leaderboard:top:{}:{}", window.as_str(), ct)
    }
}

#[async_trait::async_trait]
impl LeaderboardCache for RedisLeaderboardCache {
    async fn get_top_liked(
        &self,
        window: LeaderboardWindow,
        content_type: Option<&str>,
    ) -> Result<Option<Vec<LikeCount>>, CacheError> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| CacheError::Unavailable(e.to_string()))?;

        let key = Self::key_for(window, content_type);
        let raw: Option<String> = conn.get(&key).await.map_err(|e| {
            self.metrics.cache_error("leaderboard_get");
            CacheError::Unavailable(e.to_string())
        })?;

        match raw {
            Some(s) => match serde_json::from_str::<Vec<LikeCount>>(&s) {
                Ok(items) => {
                    self.metrics.cache_hit("leaderboard_get");
                    Ok(Some(items))
                }
                Err(_) => {
                    // Malformed -> treat as miss.
                    self.metrics.cache_miss("leaderboard_get");
                    Ok(None)
                }
            },
            None => {
                self.metrics.cache_miss("leaderboard_get");
                Ok(None)
            }
        }
    }

    async fn set_top_liked(
        &self,
        window: LeaderboardWindow,
        content_type: Option<&str>,
        items: &[LikeCount],
    ) -> Result<(), CacheError> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| CacheError::Unavailable(e.to_string()))?;

        let key = Self::key_for(window, content_type);
        let ttl_secs = self.ttl.as_secs() as usize;
        let val =
            serde_json::to_string(items).map_err(|e| CacheError::Unexpected(e.to_string()))?;

        let _: () = redis::cmd("SET")
            .arg(key)
            .arg(val)
            .arg("EX")
            .arg(ttl_secs)
            .query_async(&mut conn)
            .await
            .map_err(|e| {
                self.metrics.cache_error("leaderboard_set");
                CacheError::Unavailable(e.to_string())
            })?;

        self.metrics.cache_hit("leaderboard_set");
        Ok(())
    }
}
