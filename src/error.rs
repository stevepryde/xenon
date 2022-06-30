use crate::response::XenonResponse;
use hyper::{Body, Response};
use std::path::PathBuf;

pub type XenonResult<T> = Result<T, XenonError>;

#[derive(thiserror::Error, Debug)]
pub enum XenonError {
    #[error("Invalid port specified")]
    InvalidPort,
    #[error("Server error: {0}")]
    ServerError(String),
    #[error("WebDriver request failed: {0}")]
    RequestError(String),
    #[error("Config file not found: {0}")]
    ConfigNotFound(PathBuf),
    #[error("Error loading config from file '{0}': {1}")]
    ConfigLoadError(PathBuf, String),
    #[error("Encountered an unexpected browser in config '{0}': {1}")]
    ConfigUnexpectedBrowser(String, String),
    #[error("Error response returned to client")]
    RespondWith(XenonResponse),
    #[error("WebDriver response passed through to client")]
    ResponsePassThrough(Response<Body>),
    #[error("IO Error: {0}")]
    IOError(#[from] std::io::Error),
    #[error("No sessions available for this service")]
    NoSessionsAvailable,
}
