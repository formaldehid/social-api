use crate::state::AppState;
use axum::http::HeaderMap;
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::Serialize;

pub fn router(state: AppState) -> Router {
    let router = Router::new()
        .route("/v1/auth/validate", get(validate))
        .with_state(state);

    mock_common::health::add_routes(router)
}

async fn validate(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let token = match social_core::http::bearer_token(&headers) {
        Some(t) => t,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(ValidateErrResponse {
                    valid: false,
                    error: "invalid_token",
                }),
            )
                .into_response();
        }
    };

    match state.tokens().get(&token) {
        Some((user_id, display_name)) => (
            StatusCode::OK,
            Json(ValidateOkResponse {
                valid: true,
                user_id: user_id.clone(),
                display_name: display_name.clone(),
            }),
        )
            .into_response(),
        None => (
            StatusCode::UNAUTHORIZED,
            Json(ValidateErrResponse {
                valid: false,
                error: "invalid_token",
            }),
        )
            .into_response(),
    }
}

#[derive(Debug, Serialize)]
struct ValidateOkResponse {
    valid: bool,
    user_id: String,
    display_name: String,
}

#[derive(Debug, Serialize)]
struct ValidateErrResponse {
    valid: bool,
    error: &'static str,
}
