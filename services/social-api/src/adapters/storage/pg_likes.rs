use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use social_core::domain::ContentKey;
use sqlx::{PgPool, Row};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Clone)]
pub struct PgLikesRepository {
    pool: PgPool,
}

impl PgLikesRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn get_status(
        &self,
        user_id: &str,
        key: &ContentKey,
    ) -> anyhow::Result<Option<DateTime<Utc>>> {
        let row = sqlx::query(
            "SELECT liked_at FROM likes WHERE user_id = $1 AND content_type = $2 AND content_id = $3",
        )
            .bind(user_id)
            .bind(&key.content_type)
            .bind(key.content_id)
            .fetch_optional(&self.pool)
            .await?;

        Ok(row.map(|r| r.get::<DateTime<Utc>, _>(0)))
    }

    pub async fn get_statuses_batch(
        &self,
        user_id: &str,
        keys: &[ContentKey],
    ) -> anyhow::Result<HashMap<ContentKey, DateTime<Utc>>> {
        if keys.is_empty() {
            return Ok(HashMap::new());
        }

        let mut qb = sqlx::QueryBuilder::new(
            "SELECT content_type, content_id, liked_at FROM likes WHERE user_id = ",
        );
        qb.push_bind(user_id);
        qb.push(" AND (content_type, content_id) IN ");
        qb.push_tuples(keys, |mut tuple, k| {
            tuple.push_bind(&k.content_type);
            tuple.push_bind(k.content_id);
        });

        let rows = qb.build().fetch_all(&self.pool).await?;
        let mut out = HashMap::with_capacity(rows.len());

        for row in rows {
            let content_type: String = row.try_get("content_type")?;
            let content_id: Uuid = row.try_get("content_id")?;
            let liked_at: DateTime<Utc> = row.try_get("liked_at")?;
            out.insert(
                ContentKey {
                    content_type,
                    content_id,
                },
                liked_at,
            );
        }

        Ok(out)
    }

    pub async fn list_user_likes(
        &self,
        user_id: &str,
        content_type_filter: Option<&str>,
        cursor: Option<&Cursor>,
        limit: i64,
    ) -> anyhow::Result<(Vec<UserLikeItem>, Option<Cursor>, bool)> {
        let limit = limit.clamp(1, 100);

        let mut qb = sqlx::QueryBuilder::new(
            "SELECT content_type, content_id, liked_at FROM likes WHERE user_id = ",
        );
        qb.push_bind(user_id);

        if let Some(ct) = content_type_filter {
            qb.push(" AND content_type = ");
            qb.push_bind(ct);
        }

        if let Some(c) = cursor {
            qb.push(" AND (liked_at, content_type, content_id) < (");
            qb.push_bind(c.liked_at);
            qb.push(", ");
            qb.push_bind(&c.content_type);
            qb.push(", ");
            qb.push_bind(c.content_id);
            qb.push(")");
        }

        qb.push(" ORDER BY liked_at DESC, content_type DESC, content_id DESC");
        qb.push(" LIMIT ");
        qb.push_bind(limit + 1);

        let rows = qb.build().fetch_all(&self.pool).await?;

        let has_more = (rows.len() as i64) > limit;
        let rows = if has_more {
            &rows[..rows.len() - 1]
        } else {
            &rows
        };

        let mut items = Vec::with_capacity(rows.len());
        for row in rows {
            items.push(UserLikeItem {
                content_type: row.try_get("content_type")?,
                content_id: row.try_get("content_id")?,
                liked_at: row.try_get("liked_at")?,
            });
        }

        let next_cursor = items.last().map(|last| Cursor {
            liked_at: last.liked_at,
            content_type: last.content_type.clone(),
            content_id: last.content_id,
        });

        Ok((items, next_cursor, has_more))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cursor {
    #[serde(rename = "t")]
    pub liked_at: DateTime<Utc>,
    #[serde(rename = "ct")]
    pub content_type: String,
    #[serde(rename = "id")]
    pub content_id: Uuid,
}

impl Cursor {
    pub fn encode(&self) -> String {
        let json = serde_json::to_vec(self).expect("cursor serialize");
        URL_SAFE_NO_PAD.encode(json)
    }

    pub fn decode(s: &str) -> anyhow::Result<Self> {
        let raw = URL_SAFE_NO_PAD
            .decode(s)
            .map_err(|e| anyhow::anyhow!("invalid base64 cursor: {e}"))?;
        let cursor: Cursor = serde_json::from_slice(&raw)
            .map_err(|e| anyhow::anyhow!("invalid json cursor: {e}"))?;
        Ok(cursor)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserLikeItem {
    pub content_type: String,
    pub content_id: Uuid,
    pub liked_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_roundtrip() {
        let c = Cursor {
            liked_at: "2026-02-02T17:00:00Z".parse::<DateTime<Utc>>().unwrap(),
            content_type: "post".to_string(),
            content_id: Uuid::nil(),
        };

        let enc = c.encode();
        let dec = Cursor::decode(&enc).unwrap();
        assert_eq!(dec.content_type, "post");
        assert_eq!(dec.content_id, Uuid::nil());
        assert_eq!(dec.liked_at, c.liked_at);
    }
}
