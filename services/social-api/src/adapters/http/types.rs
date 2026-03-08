use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Deserialize)]
pub struct LikeRequest {
    pub content_type: String,
    pub content_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct LikeResponse {
    pub liked: bool,
    pub already_existed: bool,
    pub count: i64,
    pub liked_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UnlikeResponse {
    pub liked: bool,
    pub was_liked: bool,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CountResponse {
    pub content_type: String,
    pub content_id: Uuid,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusResponse {
    pub liked: bool,
    pub liked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BatchItemsRequest {
    pub items: Vec<ContentRef>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ContentRef {
    pub content_type: String,
    pub content_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct BatchCountsResponse {
    pub results: Vec<CountResponse>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BatchStatusesResponse {
    pub results: Vec<BatchStatusResult>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BatchStatusResult {
    pub content_type: String,
    pub content_id: Uuid,
    pub liked: bool,
    pub liked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UserLikesResponse {
    pub items: Vec<UserLikeItemResponse>,
    pub next_cursor: Option<String>,
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct UserLikeItemResponse {
    pub content_type: String,
    pub content_id: Uuid,
    pub liked_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UserLikesQuery {
    pub content_type: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HealthLiveResponse {
    pub status: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct HealthReadyResponse {
    pub ready: bool,
    pub checks: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TopLikedQuery {
    pub content_type: Option<String>,
    pub window: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TopLikedResponse {
    pub window: String,
    pub content_type: Option<String>,
    pub items: Vec<TopLikedItem>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TopLikedItem {
    pub content_type: String,
    pub content_id: Uuid,
    pub count: i64,
}
