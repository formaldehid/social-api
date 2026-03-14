use anyhow::{anyhow, Context, Result};
use std::{collections::HashMap, env, str::FromStr};
use url::Url;

#[derive(Debug, Clone)]
pub struct Settings {
    // Required
    pub database_url: String,
    pub read_database_url: String,
    pub redis_url: String,
    pub http_port: u16,

    // Required
    pub profile_api_url: Url,
    pub content_api_urls: HashMap<String, Url>,

    // Optional
    pub log_level: String,
    pub rust_log: String,
    pub db_max_connections: u32,
    pub db_min_connections: u32,
    pub db_acquire_timeout_secs: u64,
    pub redis_pool_size: usize,
    pub rate_limit_write_per_minute: u64,
    pub rate_limit_read_per_minute: u64,
    pub cache_ttl_like_counts_secs: u64,
    pub cache_ttl_content_validation_secs: u64,
    pub cache_ttl_user_status_secs: u64,
    pub circuit_breaker_failure_threshold: u16,
    pub circuit_breaker_recovery_timeout_secs: u64,
    pub circuit_breaker_success_threshold: u16,
    pub shutdown_timeout_secs: u64,
    pub sse_heartbeat_interval_secs: u64,
    pub leaderboard_refresh_interval_secs: u64,
}

impl Settings {
    pub fn from_env() -> Result<Self> {
        fn required(name: &str) -> Result<String> {
            env::var(name).with_context(|| format!("{name} not set"))
        }

        fn opt(name: &str, default: &str) -> String {
            env::var(name).unwrap_or_else(|_| default.to_string())
        }

        fn opt_parse<T>(name: &str, default: T) -> Result<T>
        where
            T: FromStr,
            T::Err: std::fmt::Display,
        {
            match env::var(name) {
                Ok(v) => v
                    .parse::<T>()
                    .map_err(|e| anyhow!("{name} must be valid: {e}")),
                Err(_) => Ok(default),
            }
        }

        fn required_parse<T>(name: &str) -> Result<T>
        where
            T: FromStr,
            T::Err: std::fmt::Display,
        {
            required(name)?
                .parse::<T>()
                .map_err(|e| anyhow!("{name} must be valid: {e}"))
        }

        fn required_url(name: &str) -> Result<Url> {
            required_parse::<Url>(name).with_context(|| format!("{name} must be a valid URL"))
        }

        let content_api_urls = parse_content_api_urls_from_env()?;

        Ok(Self {
            database_url: required("DATABASE_URL")?,
            read_database_url: required("READ_DATABASE_URL")?,
            redis_url: required("REDIS_URL")?,
            http_port: required_parse::<u16>("HTTP_PORT")
                .context("HTTP_PORT must be a valid u16")?,

            profile_api_url: required_url("PROFILE_API_URL")?,
            content_api_urls,

            log_level: opt("LOG_LEVEL", "info"),
            rust_log: opt("RUST_LOG", "social_api=debug"),

            db_max_connections: opt_parse("DB_MAX_CONNECTIONS", 20u32)?,
            db_min_connections: opt_parse("DB_MIN_CONNECTIONS", 5u32)?,
            db_acquire_timeout_secs: opt_parse("DB_ACQUIRE_TIMEOUT_SECS", 5u64)?,
            redis_pool_size: opt_parse("REDIS_POOL_SIZE", 10usize)?,

            rate_limit_write_per_minute: opt_parse("RATE_LIMIT_WRITE_PER_MINUTE", 30u64)?,
            rate_limit_read_per_minute: opt_parse("RATE_LIMIT_READ_PER_MINUTE", 1000u64)?,

            cache_ttl_like_counts_secs: opt_parse("CACHE_TTL_LIKE_COUNTS_SECS", 300u64)?,
            cache_ttl_content_validation_secs: opt_parse(
                "CACHE_TTL_CONTENT_VALIDATION_SECS",
                3600u64,
            )?,
            cache_ttl_user_status_secs: opt_parse("CACHE_TTL_USER_STATUS_SECS", 60u64)?,

            circuit_breaker_failure_threshold: opt_parse(
                "CIRCUIT_BREAKER_FAILURE_THRESHOLD",
                5u16,
            )?,
            circuit_breaker_recovery_timeout_secs: opt_parse(
                "CIRCUIT_BREAKER_RECOVERY_TIMEOUT_SECS",
                30u64,
            )?,
            circuit_breaker_success_threshold: opt_parse(
                "CIRCUIT_BREAKER_SUCCESS_THRESHOLD",
                3u16,
            )?,

            shutdown_timeout_secs: opt_parse("SHUTDOWN_TIMEOUT_SECS", 30u64)?,
            sse_heartbeat_interval_secs: opt_parse("SSE_HEARTBEAT_INTERVAL_SECS", 15u64)?,
            leaderboard_refresh_interval_secs: opt_parse(
                "LEADERBOARD_REFRESH_INTERVAL_SECS",
                60u64,
            )?,
        })
    }
}

/// Populates registry from env vars: CONTENT_API_<TYPE>_URL
fn parse_content_api_urls_from_env() -> Result<HashMap<String, Url>> {
    let mut map = HashMap::new();

    for (k, v) in env::vars() {
        if let Some(content_type) = k
            .strip_prefix("CONTENT_API_")
            .and_then(|s| s.strip_suffix("_URL"))
        {
            let url: Url = v
                .parse()
                .with_context(|| format!("{k} must be a valid URL"))?;
            map.insert(content_type.to_ascii_lowercase(), url);
        }
    }

    if map.is_empty() {
        return Err(anyhow!(
            "no CONTENT_API_*_URL variables found; at least one is required"
        ));
    }

    Ok(map)
}
