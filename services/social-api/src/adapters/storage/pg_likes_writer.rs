use chrono::{DateTime, Utc};
use social_core::domain::ContentKey;
use sqlx::{PgPool, Row};

#[derive(Clone)]
pub struct PgLikesWriter {
    pool: PgPool,
}

#[derive(Debug, Clone)]
pub struct LikeWriteResult {
    pub already_existed: bool,
    pub liked_at: DateTime<Utc>,
    pub count: i64,
    pub seq: i64,
}

#[derive(Debug, Clone)]
pub struct UnlikeWriteResult {
    pub was_liked: bool,
    pub count: i64,
    pub seq: i64,
}

impl PgLikesWriter {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Inserts a like if missing.
    ///
    /// Returns:
    /// - `already_existed=true` if the user had already liked the item.
    /// - `liked_at` is stable across retries (idempotent).
    /// - `count/seq` are returned from the `like_counts` row.
    pub async fn like(&self, user_id: &str, key: &ContentKey) -> anyhow::Result<LikeWriteResult> {
        let mut tx = self.pool.begin().await?;

        // Try to insert.
        let inserted = sqlx::query(
            "INSERT INTO likes (user_id, content_type, content_id, liked_at)
             VALUES ($1, $2, $3, NOW())
             ON CONFLICT (user_id, content_type, content_id) DO NOTHING
             RETURNING liked_at",
        )
        .bind(user_id)
        .bind(&key.content_type)
        .bind(key.content_id)
        .fetch_optional(&mut *tx)
        .await?;

        let (already_existed, liked_at) = match inserted {
            Some(row) => (false, row.get::<DateTime<Utc>, _>("liked_at")),
            None => {
                // Idempotent retry: keep original liked_at.
                let row = sqlx::query(
                    "SELECT liked_at FROM likes
                     WHERE user_id = $1 AND content_type = $2 AND content_id = $3",
                )
                .bind(user_id)
                .bind(&key.content_type)
                .bind(key.content_id)
                .fetch_one(&mut *tx)
                .await?;
                (true, row.get::<DateTime<Utc>, _>("liked_at"))
            }
        };

        let (count, seq) = if !already_existed {
            // Maintain the fast count row.
            let row = sqlx::query(
                "INSERT INTO like_counts (content_type, content_id, count, seq)
                 VALUES ($1, $2, 1, 1)
                 ON CONFLICT (content_type, content_id) DO UPDATE
                 SET count = like_counts.count + 1,
                     seq   = like_counts.seq + 1
                 RETURNING count, seq",
            )
            .bind(&key.content_type)
            .bind(key.content_id)
            .fetch_one(&mut *tx)
            .await?;

            // Update hourly bucket (for future leaderboard queries).
            let _ = sqlx::query(
                "INSERT INTO like_buckets_hourly (bucket_start, content_type, content_id, count)
                 VALUES (date_trunc('hour', $1), $2, $3, 1)
                 ON CONFLICT (bucket_start, content_type, content_id) DO UPDATE
                 SET count = like_buckets_hourly.count + 1",
            )
            .bind(liked_at)
            .bind(&key.content_type)
            .bind(key.content_id)
            .execute(&mut *tx)
            .await?;

            (row.get::<i64, _>("count"), row.get::<i64, _>("seq"))
        } else {
            let row = sqlx::query(
                "SELECT count, seq FROM like_counts WHERE content_type = $1 AND content_id = $2",
            )
            .bind(&key.content_type)
            .bind(key.content_id)
            .fetch_optional(&mut *tx)
            .await?;

            match row {
                Some(r) => (r.get::<i64, _>("count"), r.get::<i64, _>("seq")),
                None => (0, 0),
            }
        };

        tx.commit().await?;

        Ok(LikeWriteResult {
            already_existed,
            liked_at,
            count,
            seq,
        })
    }

    /// Deletes a like if present.
    ///
    /// Returns:
    /// - `was_liked=true` if a like existed and was removed.
    /// - `count/seq` reflect the `like_counts` row after the operation.
    pub async fn unlike(
        &self,
        user_id: &str,
        key: &ContentKey,
    ) -> anyhow::Result<UnlikeWriteResult> {
        let mut tx = self.pool.begin().await?;

        let deleted = sqlx::query(
            "DELETE FROM likes
             WHERE user_id = $1 AND content_type = $2 AND content_id = $3
             RETURNING liked_at",
        )
        .bind(user_id)
        .bind(&key.content_type)
        .bind(key.content_id)
        .fetch_optional(&mut *tx)
        .await?;

        let (was_liked, liked_at) = match deleted {
            Some(row) => (true, Some(row.get::<DateTime<Utc>, _>("liked_at"))),
            None => (false, None),
        };

        let (count, seq) = if was_liked {
            // Decrement count + bump seq. Protect against negative counts.
            let row = sqlx::query(
                "INSERT INTO like_counts (content_type, content_id, count, seq)
                 VALUES ($1, $2, 0, 1)
                 ON CONFLICT (content_type, content_id) DO UPDATE
                 SET count = GREATEST(like_counts.count - 1, 0),
                     seq   = like_counts.seq + 1
                 RETURNING count, seq",
            )
            .bind(&key.content_type)
            .bind(key.content_id)
            .fetch_one(&mut *tx)
            .await?;

            if let Some(ts) = liked_at {
                let _ = sqlx::query(
                    "INSERT INTO like_buckets_hourly (bucket_start, content_type, content_id, count)
                     VALUES (date_trunc('hour', $1), $2, $3, 0)
                     ON CONFLICT (bucket_start, content_type, content_id) DO UPDATE
                     SET count = GREATEST(like_buckets_hourly.count - 1, 0)",
                )
                .bind(ts)
                .bind(&key.content_type)
                .bind(key.content_id)
                .execute(&mut *tx)
                .await?;
            }

            (row.get::<i64, _>("count"), row.get::<i64, _>("seq"))
        } else {
            let row = sqlx::query(
                "SELECT count, seq FROM like_counts WHERE content_type = $1 AND content_id = $2",
            )
            .bind(&key.content_type)
            .bind(key.content_id)
            .fetch_optional(&mut *tx)
            .await?;

            match row {
                Some(r) => (r.get::<i64, _>("count"), r.get::<i64, _>("seq")),
                None => (0, 0),
            }
        };

        tx.commit().await?;

        Ok(UnlikeWriteResult {
            was_liked,
            count,
            seq,
        })
    }
}
