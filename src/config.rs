use crate::session::Capabilities;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct BrowserConfig {
    name: String,
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
        true
    }
}

#[derive(Debug, Default)]
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
