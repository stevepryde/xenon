use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ManagerError {
    #[error("download failed: {0}")]
    Download(String),

    #[error("extract failed: {0}")]
    Extract(String),

    #[error("could not detect installed {browser}: {hint}")]
    LocalBrowserNotFound {
        browser: &'static str,
        hint: &'static str,
    },

    #[error("unsupported browser: {0} (manager supports chrome, firefox, edge, safari)")]
    UnsupportedBrowser(String),

    #[error("offline mode and driver not present in cache: {0}")]
    Offline(PathBuf),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("http: {0}")]
    Http(String),

    #[error("parse error: {0}")]
    Parse(String),
}

impl From<reqwest::Error> for ManagerError {
    fn from(e: reqwest::Error) -> Self {
        ManagerError::Http(e.to_string())
    }
}
