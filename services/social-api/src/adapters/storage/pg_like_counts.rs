use social_core::{
    domain::ContentKey,
    ports::{LikeCountsRepository, StorageError},
};
use sqlx::{postgres::PgRow, PgPool, Row};
use uuid::Uuid;

#[derive(Clone)]
pub struct PgLikeCountsRepository {
    pool: PgPool,
}

impl PgLikeCountsRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl LikeCountsRepository for PgLikeCountsRepository {
    async fn get_count(&self, key: &ContentKey) -> Result<i64, StorageError> {
        let row: Option<PgRow> = sqlx::query(
            "SELECT count FROM like_counts WHERE content_type = $1 AND content_id = $2",
        )
        .bind(&key.content_type)
        .bind(key.content_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::Unavailable(e.to_string()))?;

        Ok(row.map(|r| r.get::<i64, _>(0)).unwrap_or(0))
    }

    async fn get_counts(
        &self,
        keys: &[ContentKey],
    ) -> Result<Vec<(ContentKey, i64)>, StorageError> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }

        let mut qb = sqlx::QueryBuilder::new(
            "SELECT content_type, content_id, count FROM like_counts WHERE (content_type, content_id) IN ",
        );

        qb.push_tuples(keys, |mut tuple, k| {
            tuple.push_bind(&k.content_type);
            tuple.push_bind(k.content_id);
        });

        let rows = qb
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(|e| StorageError::Unavailable(e.to_string()))?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let content_type: String = row.try_get("content_type").unwrap_or_default();
            let content_id: Uuid = row.try_get("content_id").unwrap_or_else(|_| Uuid::nil());
            let count: i64 = row.try_get("count").unwrap_or(0);
            out.push((
                ContentKey {
                    content_type,
                    content_id,
                },
                count,
            ));
        }

        Ok(out)
    }
}
