use crate::{
    domain::ContentKey,
    ports::{CacheError, LikeCountsCache, LikeCountsRepository, StorageError},
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum LikeCountsError {
    #[error("storage unavailable")]
    StorageUnavailable,
    #[error("unexpected error")]
    Unexpected,
}

impl From<StorageError> for LikeCountsError {
    fn from(e: StorageError) -> Self {
        match e {
            StorageError::Unavailable(_) => LikeCountsError::StorageUnavailable,
            StorageError::Unexpected(_) => LikeCountsError::Unexpected,
        }
    }
}

impl From<CacheError> for LikeCountsError {
    fn from(_e: CacheError) -> Self {
        // Cache failures must not break reads; callers handle them by falling back to DB.
        LikeCountsError::Unexpected
    }
}

/// Use-cases for Like counts.
///
/// - Read path prefers cache.
/// - If cache is unavailable, falls back to repository (DB) as a degraded mode.
///
/// This aligns with the spec requirement that count endpoints are served from cache,
/// but continue operating when Redis is unavailable.
#[derive(Clone)]
pub struct LikeCountsService<C, R> {
    cache: C,
    repo: R,
}

impl<C, R> LikeCountsService<C, R>
where
    C: LikeCountsCache,
    R: LikeCountsRepository,
{
    pub fn new(cache: C, repo: R) -> Self {
        Self { cache, repo }
    }

    pub async fn get_count(&self, key: &ContentKey) -> Result<i64, LikeCountsError> {
        match self.cache.get_count(key).await {
            Ok(Some(count)) => return Ok(count),
            Ok(None) => {
                // Cache miss -> DB, then fill cache (best-effort).
            }
            Err(_e) => {
                // Cache error -> degraded mode, fall back to DB.
            }
        }

        let count = self.repo.get_count(key).await?;
        let _ = self.cache.set_count(key, count).await; // best-effort
        Ok(count)
    }

    /// Batch gets counts.
    ///
    /// Returns counts in the same order as input.
    pub async fn get_counts(&self, keys: &[ContentKey]) -> Result<Vec<i64>, LikeCountsError> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }

        let cached = match self.cache.get_counts(keys).await {
            Ok(v) => v,
            Err(_) => {
                let from_db = self.repo.get_counts(keys).await?;
                return Ok(project_in_order(keys, from_db));
            }
        };

        let mut misses = Vec::new();
        for (idx, item) in cached.iter().enumerate() {
            if item.is_none() {
                misses.push(idx);
            }
        }

        if misses.is_empty() {
            return Ok(cached.into_iter().map(|x| x.unwrap_or(0)).collect());
        }

        let miss_keys: Vec<ContentKey> = misses.iter().map(|&i| keys[i].clone()).collect();
        let from_db = self.repo.get_counts(&miss_keys).await?;
        let _ = self.cache.set_counts(&from_db).await; // best-effort

        let mut out: Vec<i64> = cached.into_iter().map(|x| x.unwrap_or(0)).collect();
        let filled = to_map(from_db);
        for (pos, key) in misses.into_iter().zip(miss_keys) {
            out[pos] = *filled.get(&key).unwrap_or(&0);
        }
        Ok(out)
    }
}

fn to_map(items: Vec<(ContentKey, i64)>) -> std::collections::HashMap<ContentKey, i64> {
    items.into_iter().collect()
}

fn project_in_order(keys: &[ContentKey], items: Vec<(ContentKey, i64)>) -> Vec<i64> {
    let map = to_map(items);
    keys.iter().map(|k| *map.get(k).unwrap_or(&0)).collect()
}
