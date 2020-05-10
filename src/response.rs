use hyper::{Body, StatusCode};
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE", tag = "code", content = "message")]
pub enum XenonResponse {
    PathNotFound(String),
    MethodNotFound(String),
    SessionNotFound(String),
    ErrorCreatingSession(String),
    InternalServerError(String),
}

impl XenonResponse {
    pub fn status(&self) -> StatusCode {
        match self {
            XenonResponse::PathNotFound(_) | XenonResponse::MethodNotFound(_) => {
                StatusCode::BAD_REQUEST
            }
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl Into<Body> for XenonResponse {
    fn into(self) -> Body {
        Body::from(
            serde_json::to_string(&self)
                .unwrap_or_else(|e| format!("JSON error message conversion failed: {}", e)),
        )
    }
}
