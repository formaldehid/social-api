use axum::{http::StatusCode, response::IntoResponse, Json};
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Serialize)]
pub struct ErrorEnvelope {
    pub error: ErrorBody,
}

#[derive(Debug, Clone, Serialize)]
pub struct ErrorBody {
    pub code: String,
    pub message: String,
    pub request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

#[derive(Debug, Clone)]
pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
    request_id: String,
    details: Option<Value>,
}

impl ApiError {
    pub fn new(
        status: StatusCode,
        code: &'static str,
        message: impl Into<String>,
        request_id: String,
    ) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            request_id,
            details: None,
        }
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }

    pub fn unauthorized(request_id: String) -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            "UNAUTHORIZED",
            "Missing, malformed, or invalid token",
            request_id,
        )
    }

    pub fn content_type_unknown(content_type: &str, request_id: String) -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            "CONTENT_TYPE_UNKNOWN",
            "content_type not in configured registry",
            request_id,
        )
        .with_details(serde_json::json!({ "content_type": content_type }))
    }

    pub fn invalid_content_id(content_id: &str, request_id: String) -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            "INVALID_CONTENT_ID",
            "content_id is not a valid UUID v4",
            request_id,
        )
        .with_details(serde_json::json!({ "content_id": content_id }))
    }

    pub fn content_not_found(content_type: &str, content_id: &str, request_id: String) -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            "CONTENT_NOT_FOUND",
            "Content item does not exist or has been removed",
            request_id,
        )
        .with_details(serde_json::json!({
            "content_type": content_type,
            "content_id": content_id
        }))
    }

    pub fn batch_too_large(max: usize, request_id: String) -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            "BATCH_TOO_LARGE",
            format!("Batch exceeds {max} items"),
            request_id,
        )
        .with_details(serde_json::json!({ "max": max }))
    }

    pub fn invalid_cursor(request_id: String) -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            "INVALID_CURSOR",
            "Pagination cursor is malformed or expired",
            request_id,
        )
    }

    pub fn dependency_unavailable(message: impl Into<String>, request_id: String) -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "DEPENDENCY_UNAVAILABLE",
            message,
            request_id,
        )
    }

    pub fn not_implemented(message: impl Into<String>, request_id: String) -> Self {
        Self::new(
            StatusCode::NOT_IMPLEMENTED,
            "NOT_IMPLEMENTED",
            message,
            request_id,
        )
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let body = ErrorEnvelope {
            error: ErrorBody {
                code: self.code.to_string(),
                message: self.message,
                request_id: self.request_id,
                details: self.details,
            },
        };
        (self.status, Json(body)).into_response()
    }
}
