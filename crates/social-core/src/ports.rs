use crate::domain::{ContentKey, LeaderboardWindow, LikeCount, UserIdentity};
use async_trait::async_trait;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("unauthorized")]
    Unauthorized,
    #[error("dependency unavailable: {0}")]
    DependencyUnavailable(String),
}

#[async_trait]
pub trait AuthProvider: Send + Sync {
    async fn validate_token(&self, bearer_token: &str) -> Result<UserIdentity, AuthError>;
}

#[derive(Debug, Error)]
pub enum ContentError {
    #[error("unknown content type: {0}")]
    UnknownContentType(String),
    #[error("dependency unavailable: {0}")]
    DependencyUnavailable(String),
}

#[async_trait]
pub trait ContentCatalog: Send + Sync {
    /// Returns Ok(true) if the item exists, Ok(false) if it does not.
    async fn exists(&self, key: &ContentKey) -> Result<bool, ContentError>;
}

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("storage unavailable: {0}")]
    Unavailable(String),
    #[error("unexpected storage error: {0}")]
    Unexpected(String),
}

#[async_trait]
pub trait LikeCountsRepository: Send + Sync {
    async fn get_count(&self, key: &ContentKey) -> Result<i64, StorageError>;
    async fn get_counts(&self, keys: &[ContentKey])
        -> Result<Vec<(ContentKey, i64)>, StorageError>;
}

#[derive(Debug, Error)]
pub enum CacheError {
    #[error("cache unavailable: {0}")]
    Unavailable(String),
    #[error("unexpected cache error: {0}")]
    Unexpected(String),
}

#[async_trait]
pub trait LikeCountsCache: Send + Sync {
    async fn get_count(&self, key: &ContentKey) -> Result<Option<i64>, CacheError>;
    async fn set_count(&self, key: &ContentKey, count: i64) -> Result<(), CacheError>;
    async fn get_counts(&self, keys: &[ContentKey]) -> Result<Vec<Option<i64>>, CacheError>;
    async fn set_counts(&self, items: &[(ContentKey, i64)]) -> Result<(), CacheError>;
}

/// Backing store for the leaderboard.
#[async_trait]
pub trait LeaderboardRepository: Send + Sync {
    /// Returns the most-liked content items for the given time window.
    async fn top_liked(
        &self,
        window: LeaderboardWindow,
        content_type: Option<&str>,
        limit: u32,
    ) -> Result<Vec<LikeCount>, StorageError>;
}

/// Cache for leaderboard payloads.
#[async_trait]
pub trait LeaderboardCache: Send + Sync {
    async fn get_top_liked(
        &self,
        window: LeaderboardWindow,
        content_type: Option<&str>,
    ) -> Result<Option<Vec<LikeCount>>, CacheError>;

    async fn set_top_liked(
        &self,
        window: LeaderboardWindow,
        content_type: Option<&str>,
        items: &[LikeCount],
    ) -> Result<(), CacheError>;
}
