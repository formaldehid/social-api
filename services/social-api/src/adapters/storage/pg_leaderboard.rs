use chrono::Utc;
use social_core::{
    domain::{LeaderboardWindow, LikeCount},
    ports::{LeaderboardRepository, StorageError},
};
use sqlx::{PgPool, Row};
use uuid::Uuid;

#[derive(Clone)]
pub struct PgLeaderboardRepository {
    pool: PgPool,
}

impl PgLeaderboardRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl LeaderboardRepository for PgLeaderboardRepository {
    async fn top_liked(
        &self,
        window: LeaderboardWindow,
        content_type: Option<&str>,
        limit: u32,
    ) -> Result<Vec<LikeCount>, StorageError> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let limit = limit.min(50) as i64;

        let rows = match window {
            LeaderboardWindow::All => {
                let mut qb = sqlx::QueryBuilder::new(
                    "SELECT content_type, content_id, count FROM like_counts WHERE count > 0",
                );

                if let Some(ct) = content_type {
                    qb.push(" AND content_type = ");
                    qb.push_bind(ct);
                }

                qb.push(" ORDER BY count DESC LIMIT ");
                qb.push_bind(limit);

                qb.build()
                    .fetch_all(&self.pool)
                    .await
                    .map_err(|e| StorageError::Unavailable(e.to_string()))?
            }
            _ => {
                let since = window
                    .since(Utc::now())
                    .expect("time-bounded window must have since");

                let mut qb = sqlx::QueryBuilder::new(
                    "SELECT content_type, content_id, SUM(count)::BIGINT AS count \
                     FROM like_buckets_hourly \
                     WHERE bucket_start >= ",
                );
                qb.push_bind(since);

                if let Some(ct) = content_type {
                    qb.push(" AND content_type = ");
                    qb.push_bind(ct);
                }

                qb.push(
                    " GROUP BY content_type, content_id \
                      HAVING SUM(count) > 0 \
                      ORDER BY count DESC \
                      LIMIT ",
                );
                qb.push_bind(limit);

                qb.build()
                    .fetch_all(&self.pool)
                    .await
                    .map_err(|e| StorageError::Unavailable(e.to_string()))?
            }
        };

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let content_type: String = row.try_get("content_type").unwrap_or_default();
            let content_id: Uuid = row.try_get("content_id").unwrap_or_else(|_| Uuid::nil());
            let count: i64 = row.try_get("count").unwrap_or(0);
            out.push(LikeCount {
                content_type,
                content_id,
                count,
            });
        }

        Ok(out)
    }
}
