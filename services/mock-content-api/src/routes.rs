use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::Serialize;
use uuid::Uuid;

pub fn router(state: AppState) -> Router {
    let router = Router::new()
        .route("/v1/{content_type}/{content_id}", get(get_content))
        .with_state(state);

    mock_common::health::add_routes(router)
}

#[derive(Debug, Serialize)]
pub struct ContentItem {
    id: Uuid,
    title: String,
    content_type: String,
}

async fn get_content(
    State(state): State<AppState>,
    Path((content_type, content_id)): Path<(String, String)>,
) -> Response {
    // If called with wrong content type, behave like "not found"
    if content_type != state.content_type() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "not_found" })),
        )
            .into_response();
    }

    let id = match Uuid::parse_str(&content_id) {
        Ok(u) => u,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "invalid_uuid" })),
            )
                .into_response();
        }
    };

    match state.get_title(&id) {
        Some(title) => (
            StatusCode::OK,
            Json(ContentItem {
                id,
                title: title.to_string(),
                content_type,
            }),
        )
            .into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "not_found" })),
        )
            .into_response(),
    }
}
