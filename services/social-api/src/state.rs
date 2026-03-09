use crate::{
    adapters::{
        external::{content_catalog::HttpContentCatalog, profile_auth::ProfileHttpAuth},
        storage::{
            pg_leaderboard::PgLeaderboardRepository, pg_like_counts::PgLikeCountsRepository,
            pg_likes::PgLikesRepository, pg_likes_writer::PgLikesWriter,
            redis_content_validation::RedisContentValidationCache,
            redis_leaderboard_cache::RedisLeaderboardCache,
            redis_like_counts::RedisLikeCountsCache, redis_rate_limiter::RedisRateLimiter,
        },
    },
    infra::{config::Settings, metrics::Metrics},
};
use social_core::circuit_breaker::{CircuitBreaker, CircuitBreakerConfig};
use social_core::usecases::{LeaderboardService, LikeCountsService};
use sqlx::{postgres::PgPoolOptions, PgPool};
use std::time::Duration;
use std::{collections::HashMap, sync::Arc};
use url::Url;

pub type LikeCountsSvc = LikeCountsService<RedisLikeCountsCache, PgLikeCountsRepository>;
pub type LeaderboardSvc = LeaderboardService<RedisLeaderboardCache, PgLeaderboardRepository>;

#[derive(Clone)]
pub struct AppState {
    pub settings: Settings,
    pub db_writer: PgPool,
    pub db_reader: PgPool,
    pub redis: deadpool_redis::Pool,
    pub http_client: reqwest::Client,
    pub metrics: Arc<Metrics>,

    pub content_registry: Arc<HashMap<String, Url>>,
    pub auth: ProfileHttpAuth,
    pub content_catalog: HttpContentCatalog,

    pub like_counts: LikeCountsSvc,
    pub like_counts_cache: RedisLikeCountsCache,
    pub likes_repo: PgLikesRepository,
    pub likes_writer: PgLikesWriter,
    pub rate_limiter: RedisRateLimiter,

    pub leaderboard: LeaderboardSvc,
    pub leaderboard_cache: RedisLeaderboardCache,
    pub leaderboard_repo: PgLeaderboardRepository,
}

impl AppState {
    pub async fn try_new(settings: Settings) -> anyhow::Result<Self> {
        let metrics = Metrics::new()?;

        let http_client = reqwest::Client::builder()
            .user_agent("social-api/0.1")
            .timeout(Duration::from_secs(2))
            .build()?;

        let db_writer = PgPoolOptions::new()
            .max_connections(settings.db_max_connections)
            .min_connections(settings.db_min_connections)
            .acquire_timeout(Duration::from_secs(settings.db_acquire_timeout_secs))
            .connect(&settings.database_url)
            .await?;

        let db_reader = PgPoolOptions::new()
            .max_connections(settings.db_max_connections)
            .min_connections(settings.db_min_connections)
            .acquire_timeout(Duration::from_secs(settings.db_acquire_timeout_secs))
            .connect(&settings.read_database_url)
            .await?;

        // Run migrations once on the writer.
        sqlx::migrate!("./migrations").run(&db_writer).await?;

        let mut redis_cfg = deadpool_redis::Config::from_url(settings.redis_url.clone());
        redis_cfg.pool = Some(deadpool_redis::PoolConfig::new(settings.redis_pool_size));
        let redis = redis_cfg.create_pool(Some(deadpool_redis::Runtime::Tokio1))?;

        let rate_limiter = RedisRateLimiter::new(redis.clone(), metrics.clone());

        let content_registry = Arc::new(settings.content_api_urls.clone());

        // Circuit breaker config is shared across all external services.
        let cb_cfg = CircuitBreakerConfig {
            failure_threshold: settings.circuit_breaker_failure_threshold as u32,
            recovery_timeout: Duration::from_secs(settings.circuit_breaker_recovery_timeout_secs),
            success_threshold: settings.circuit_breaker_success_threshold as u32,
            failure_rate_window: Duration::from_secs(30),
        };

        // Per-service circuit breakers.
        let profile_cb = CircuitBreaker::new(cb_cfg.clone());

        let mut content_breakers = HashMap::new();
        for ct in content_registry.keys() {
            content_breakers.insert(ct.clone(), CircuitBreaker::new(cb_cfg.clone()));
        }
        let content_breakers = Arc::new(content_breakers);

        // Content validation cache.
        let content_validation_cache = RedisContentValidationCache::new(
            redis.clone(),
            Duration::from_secs(settings.cache_ttl_content_validation_secs),
            metrics.clone(),
        );

        let auth = ProfileHttpAuth::new(
            settings.profile_api_url.clone(),
            http_client.clone(),
            metrics.clone(),
            profile_cb,
            "profile_api",
        );
        let content_catalog = HttpContentCatalog::new(
            content_registry.clone(),
            http_client.clone(),
            metrics.clone(),
            content_validation_cache,
            content_breakers,
        );

        let like_counts_cache = RedisLikeCountsCache::new(
            redis.clone(),
            Duration::from_secs(settings.cache_ttl_like_counts_secs),
            metrics.clone(),
        );
        let counts_repo = PgLikeCountsRepository::new(db_reader.clone());
        let like_counts = LikeCountsService::new(like_counts_cache.clone(), counts_repo);

        let likes_repo = PgLikesRepository::new(db_reader.clone());
        let likes_writer = PgLikesWriter::new(db_writer.clone());

        let leaderboard_ttl_secs = settings
            .leaderboard_refresh_interval_secs
            .saturating_mul(2)
            .max(5);
        let leaderboard_cache = RedisLeaderboardCache::new(
            redis.clone(),
            Duration::from_secs(leaderboard_ttl_secs),
            metrics.clone(),
        );
        let leaderboard_repo = PgLeaderboardRepository::new(db_reader.clone());
        let leaderboard =
            LeaderboardService::new(leaderboard_cache.clone(), leaderboard_repo.clone());

        Ok(Self {
            settings,
            db_writer,
            db_reader,
            redis,
            http_client,
            metrics,
            content_registry,
            auth,
            content_catalog,
            like_counts,
            like_counts_cache,
            likes_repo,
            likes_writer,
            rate_limiter,
            leaderboard,
            leaderboard_cache,
            leaderboard_repo,
        })
    }
}
