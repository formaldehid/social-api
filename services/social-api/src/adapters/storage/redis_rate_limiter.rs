use crate::infra::metrics::Metrics;
use deadpool_redis::Pool;
use std::sync::Arc;

/// Decision returned by the rate limiter for a single request.
#[derive(Debug, Clone)]
pub struct RateLimitDecision {
    pub allowed: bool,
    pub limit: u64,
    /// Remaining requests in the current window *after* counting this request.
    pub remaining: u64,
    /// Epoch seconds when the current window resets.
    pub reset_epoch: u64,
    /// Seconds until reset (>= 0).
    pub retry_after_secs: u64,
}

#[derive(Clone)]
pub struct RedisRateLimiter {
    pool: Pool,
    metrics: Arc<Metrics>,
    window_secs: u64,
    key_ttl_secs: u64,
}

impl RedisRateLimiter {
    pub fn new(pool: Pool, metrics: Arc<Metrics>) -> Self {
        Self {
            pool,
            metrics,
            window_secs: 60,
            // Keep the key around slightly longer than the window so replicas that land
            // right at the edge don't re-create counters.
            key_ttl_secs: 61,
        }
    }

    /// Checks and consumes one token in the current fixed window.
    ///
    /// Key format: `rl:{scope}:{id}:{window_start}`.
    ///
    /// Uses Redis TIME so multiple replicas share a consistent view of the window boundaries.
    pub async fn check(
        &self,
        scope: &str,
        id: &str,
        limit_per_minute: u64,
    ) -> anyhow::Result<RateLimitDecision> {
        const LUA: &str = r#"
-- ARGV:
-- 1 scope
-- 2 id
-- 3 limit
-- 4 window_secs
-- 5 ttl_secs

local scope = ARGV[1]
local id = ARGV[2]
local limit = tonumber(ARGV[3])
local window = tonumber(ARGV[4])
local ttl = tonumber(ARGV[5])

local now = tonumber(redis.call('TIME')[1])
local window_start = now - (now % window)
local key = "rl:" .. scope .. ":" .. id .. ":" .. window_start

local current = redis.call('INCR', key)
if current == 1 then
  redis.call('EXPIRE', key, ttl)
end

local reset = window_start + window
local allowed = 0
if current <= limit then allowed = 1 end

local remaining = limit - current
if remaining < 0 then remaining = 0 end

local retry_after = reset - now
if retry_after < 0 then retry_after = 0 end

return {allowed, current, remaining, reset, limit, retry_after}
"#;

        let mut conn = self.pool.get().await.map_err(|e| {
            self.metrics.cache_error("rate_limit");
            anyhow::anyhow!("redis pool unavailable: {e}")
        })?;

        let script = redis::Script::new(LUA);

        let out: Vec<i64> = script
            .arg(scope)
            .arg(id)
            .arg(limit_per_minute as i64)
            .arg(self.window_secs as i64)
            .arg(self.key_ttl_secs as i64)
            .invoke_async(&mut conn)
            .await
            .map_err(|e| {
                self.metrics.cache_error("rate_limit");
                anyhow::anyhow!("redis eval failed: {e}")
            })?;

        // out = [allowed, current, remaining, reset, limit, retry_after]
        if out.len() < 6 {
            self.metrics.cache_error("rate_limit");
            return Err(anyhow::anyhow!(
                "redis rate limit script returned unexpected payload"
            ));
        }

        let allowed = out[0] == 1;
        let remaining = out[2].max(0) as u64;
        let reset_epoch = out[3].max(0) as u64;
        let limit = out[4].max(0) as u64;
        let retry_after_secs = out[5].max(0) as u64;

        if allowed {
            self.metrics.cache_hit("rate_limit");
        } else {
            self.metrics.cache_miss("rate_limit");
        }

        Ok(RateLimitDecision {
            allowed,
            limit,
            remaining,
            reset_epoch,
            retry_after_secs,
        })
    }
}
