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

    fn encode_value(seq: i64, count: i64) -> String {
        format!("{seq}|{count}")
    }

    fn decode_value(raw: &str) -> Option<(i64, i64)> {
        // Backward compatible with early versions that stored just the count.
        if let Some((seq_s, count_s)) = raw.split_once('|') {
            let seq = seq_s.parse::<i64>().ok()?;
            let count = count_s.parse::<i64>().ok()?;
            return Some((seq, count));
        }

        // Legacy format: only count.
        let count = raw.parse::<i64>().ok()?;
        Some((0, count))
    }

    /// Best-effort CAS update used by the write path.
    ///
    /// Stores the value as `"{seq}|{count}"` and only overwrites when `seq` is newer
    /// than (or equal to) the cached seq.
    pub async fn set_count_cas(
        &self,
        key: &ContentKey,
        count: i64,
        seq: i64,
    ) -> Result<bool, CacheError> {
        // Lua script: compare cached seq and set only if incoming seq is newer.
        // Returns 1 if updated, 0 if skipped.
        const LUA: &str = r#"
local current = redis.call('GET', KEYS[1])
local incoming_seq = tonumber(ARGV[1])
local ttl = tonumber(ARGV[3])

if current then
  local sep = string.find(current, '|')
  if sep then
    local cur_seq = tonumber(string.sub(current, 1, sep - 1)) or 0
    if cur_seq > incoming_seq then
      return 0
    end
  else
    -- legacy format: treat as seq=0
    local cur_seq = 0
    if cur_seq > incoming_seq then
      return 0
    end
  end
end

redis.call('SETEX', KEYS[1], ttl, ARGV[1] .. '|' .. ARGV[2])
return 1
"#;

        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| CacheError::Unavailable(e.to_string()))?;

        let redis_key = Self::key_for(key);
        let ttl_secs: u64 = self.ttl.as_secs();

        let script = redis::Script::new(LUA);
        let res: i32 = script
            .key(redis_key)
            .arg(seq)
            .arg(count)
            .arg(ttl_secs)
            .invoke_async(&mut conn)
            .await
            .map_err(|e| {
                self.metrics.cache_error("cas_set");
                CacheError::Unavailable(e.to_string())
            })?;

        if res == 1 {
            self.metrics.cache_hit("cas_set");
            Ok(true)
        } else {
            self.metrics.cache_miss("cas_set");
            Ok(false)
        }
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
            Some(s) => match Self::decode_value(&s) {
                Some((_seq, count)) => {
                    self.metrics.cache_hit("get");
                    Ok(Some(count))
                }
                None => {
                    // Malformed cache value -> treat as miss.
                    self.metrics.cache_miss("get");
                    Ok(None)
                }
            },
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
        // Cache warming on read-miss must not overwrite a concurrently updated value.
        // We store `seq=0` for warmed entries.
        // NOTE: `SET ... NX` is used to avoid overwriting a newer value.
        let val = Self::encode_value(0, count);
        let _: Option<String> = redis::cmd("SET")
            .arg(redis_key)
            .arg(val)
            .arg("EX")
            .arg(ttl_secs)
            .arg("NX")
            .query_async(&mut conn)
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
            .map(|opt| opt.and_then(|s| Self::decode_value(&s).map(|(_seq, c)| c)))
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
            // Like set_count(): warm cache without overwriting newer values.
            pipe.cmd("SET")
                .arg(Self::key_for(k))
                .arg(Self::encode_value(0, *count))
                .arg("EX")
                .arg(ttl_secs)
                .arg("NX")
                .ignore();
        }

        pipe.query_async::<()>(&mut conn).await.map_err(|e| {
            self.metrics.cache_error("setex");
            CacheError::Unavailable(e.to_string())
        })?;

        Ok(())
    }
}
