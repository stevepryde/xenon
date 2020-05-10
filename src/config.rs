use crate::error::XenonError;
use crate::session::Capabilities;
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
pub struct BrowserConfig {
    name: String,
    version: String,
    os: String,
    driver_path: PathBuf,
    max_sessions: u32,
}

impl BrowserConfig {
    pub fn name(&self) -> &str {
        &self.name.as_str()
    }

    pub fn driver_path(&self) -> &Path {
        &self.driver_path.as_path()
    }

    pub fn matches_capabilities(&self, capabilities: &Capabilities) -> bool {
        // TODO: implement browser name/version/os matching.
        true
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct XenonConfig {
    browsers: Vec<BrowserConfig>,
}

impl XenonConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn match_capabilities(&self, capabilities: &Capabilities) -> Option<BrowserConfig> {
        for browser in &self.browsers {
            if browser.matches_capabilities(capabilities) {
                return Some(browser.clone());
            }
        }
        None
    }
}

pub fn load_config(filename: &str) -> Result<XenonConfig, XenonError> {
    let config_path = Path::new(filename);
    if !config_path.exists() {
        return Err(XenonError::ConfigNotFound(filename.to_string()));
    }

    let config_str = std::fs::read_to_string(config_path)
        .map_err(|e| XenonError::ConfigLoadError(filename.to_string(), e.to_string()))?;
    let config = serde_yaml::from_str(&config_str)
        .map_err(|e| XenonError::ConfigLoadError(filename.to_string(), e.to_string()))?;
    Ok(config)
}
