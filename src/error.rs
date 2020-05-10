use thiserror::Error;

pub type XenonResult<T> = Result<T, XenonError>;

#[derive(Error, Debug)]
pub enum XenonError {
    #[error("WebDriver request failed")]
    RequestError(String),
    #[error("Error parsing new session params")]
    NewSessionError(String),
    #[error("Session not found")]
    SessionNotFound,
}
