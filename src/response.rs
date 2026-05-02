use axum::body::Body;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
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

impl IntoResponse for XenonResponse {
    fn into_response(self) -> Response {
        let status = self.status();
        let body: Body = self.into();
        (status, [(header::CONTENT_TYPE, "application/json")], body).into_response()
    }
}

impl From<XenonResponse> for Body {
    fn from(resp: XenonResponse) -> Self {
        // Construct WebDriver-compatible JSON output.
        let status = resp.status().as_u16();
        let (error_code, message) = match resp {
            XenonResponse::EndpointNotFound(x) => ("unknown method", x),
            XenonResponse::MethodNotFound(x) => ("unknown method", x),
            XenonResponse::SessionNotFound(x) => ("invalid session id", x),
            XenonResponse::ErrorCreatingSession(x) => ("session not created", x),
            XenonResponse::NoMatchingBrowser => (
                "session not created",
                String::from("No browser was found to match the desired capabilities"),
            ),
            XenonResponse::NoSessionsAvailable => (
                "session not created",
                String::from("Session limit reached. No available sessions"),
            ),
            XenonResponse::InternalServerError(x) => ("unknown error", x),
            XenonResponse::ErrorCreatingNode(x) => ("error creating node", x),
        };

        let json_body = serde_json::json!({
            "status": status,
            "state": error_code,
            "value": {
                "message": message,
                "error": error_code,
            }
        });

        Body::from(
            serde_json::to_string(&json_body)
                .unwrap_or_else(|e| format!("JSON error message conversion failed: {e}")),
        )
    }
}
