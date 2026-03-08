use axum::http::HeaderMap;

pub fn bearer_token(headers: &HeaderMap) -> Option<String> {
    let value = headers.get("authorization")?.to_str().ok()?;
    let value = value.strip_prefix("Bearer ")?;
    Some(value.trim().to_string())
}
