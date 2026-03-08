use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Identifies a content item across all verticals.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ContentKey {
    pub content_type: String,
    pub content_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LikeCount {
    pub content_type: String,
    pub content_id: Uuid,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LikeStatus {
    pub liked: bool,
    pub liked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserIdentity {
    pub user_id: String,
    pub display_name: Option<String>,
}
