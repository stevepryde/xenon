use hyper::{Body, StatusCode};
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE", tag = "code", content = "message")]
pub enum XenonResponse {
    EndpointNotFound(String),
    MethodNotFound(String),
    SessionNotFound(String),
    ErrorCreatingSession(String),
    NoMatchingBrowser,
    NoSessionsAvailable,
    InternalServerError(String),
    ErrorCreatingNode(String),
}

impl XenonResponse {
    pub fn status(&self) -> StatusCode {
        match self {
            XenonResponse::EndpointNotFound(_) | XenonResponse::MethodNotFound(_) => {
                StatusCode::BAD_REQUEST
            }
            XenonResponse::NoMatchingBrowser | XenonResponse::NoSessionsAvailable => {
                StatusCode::NOT_FOUND
            }
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl Into<Body> for XenonResponse {
    fn into(self) -> Body {
        // Construct WebDriver-compatible JSON output.
        let (error_code, message) = match &self {
            XenonResponse::EndpointNotFound(x) => ("unknown method", x.clone()),
            XenonResponse::MethodNotFound(x) => ("unknown method", x.clone()),
            XenonResponse::SessionNotFound(x) => ("invalid session id", x.clone()),
            XenonResponse::ErrorCreatingSession(x) => ("session not created", x.clone()),
            XenonResponse::NoMatchingBrowser => (
                "session not created",
                String::from("No browser was found to match the desired capabilities"),
            ),
            XenonResponse::NoSessionsAvailable => (
                "session not created",
                String::from("Session limit reached. No available sessions"),
            ),
            XenonResponse::InternalServerError(x) => ("unknown error", x.clone()),
            XenonResponse::ErrorCreatingNode(x) => ("error creating node", x.clone()),
        };

        let json_body = serde_json::json!({
            "status": self.status().as_u16(),
            "state": error_code,
            "value": {
                "message": message,
                "error": error_code,
            }
        });

        Body::from(
            serde_json::to_string(&json_body)
                .unwrap_or_else(|e| format!("JSON error message conversion failed: {}", e)),
        )
    }
}
