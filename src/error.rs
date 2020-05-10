use crate::response::XenonResponse;
use thiserror::Error;

pub type XenonResult<T> = Result<T, XenonError>;

#[derive(Error, Debug)]
pub enum XenonError {
    #[error("Invalid port specified")]
    InvalidPort,
    #[error("Server error: {0}")]
    ServerError(String),
    #[error("WebDriver request failed: {0}")]
    RequestError(String),
    #[error("Config file not found: {0}")]
    ConfigNotFound(String),
    #[error("Error loading config from file '{0}': {1}")]
    ConfigLoadError(String, String),
    #[error("Error response returned to client")]
    RespondWith(XenonResponse),
}
