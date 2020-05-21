use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub fn default_sessions_per_driver() -> u32 {
    1
}

pub fn default_max_sessions() -> u32 {
    5
}

#[derive(Debug, Clone, Deserialize)]
pub struct BrowserConfig {
    name: String,
    version: Option<String>,
    os: Option<String>,
    driver_path: PathBuf,
    #[serde(default = "default_sessions_per_driver")]
    sessions_per_driver: u32,
    #[serde(default = "default_max_sessions")]
    max_sessions: u32,
}

impl BrowserConfig {
    pub fn name(&self) -> &str {
        &self.name.as_str()
    }

    pub fn driver_path(&self) -> &Path {
        &self.driver_path.as_path()
    }

    pub fn sessions_per_driver(&self) -> u32 {
        self.sessions_per_driver
    }

    pub fn max_sessions(&self) -> u32 {
        self.max_sessions
    }

    pub fn matches_capabilities(&self, capabilities: &Capabilities) -> bool {
        // TODO: match version and OS as well.
        self.name.to_lowercase() == capabilities.browser_name().to_lowercase()
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserMatch {
    browser_name: String,
    browser_version: Option<String>,
    platform_name: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    always_match: BrowserMatch,
}

impl Capabilities {
    pub fn browser_name(&self) -> &str {
        &self.always_match.browser_name
    }

    pub fn browser_version(&self) -> &Option<String> {
        &self.always_match.browser_version
    }

    pub fn platform_name(&self) -> &Option<String> {
        &self.always_match.platform_name
    }
}
