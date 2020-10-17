use crate::error::XenonError;
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
    /// driver_path always contains a path to a webdriver
    /// It may be configured value or a default one.
    driver_path: Option<PathBuf>,
    args: Option<Vec<String>>,
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
        match &self.driver_path {
            Some(path) => path.as_path(),
            _ => {
                unreachable!();
            }
        }
    }

    pub fn args(&self) -> &Option<Vec<String>> {
        &self.args
    }

    pub fn sessions_per_driver(&self) -> u32 {
        self.sessions_per_driver
    }

    pub fn max_sessions(&self) -> u32 {
        self.max_sessions
    }

    /// Does this browser match the capabilities we are searching for?
    /// Browser name must match.
    /// For browser version and platform, the following rules apply:
    /// 1. If the required browser version or platform is specified,
    ///    the system will only consider it a match if those are both
    ///    known and identical.
    /// 2. If the actual version or platform is not specified on the browser
    ///    object, it is considered unknown and thus will only match if the
    ///    version or platform is not required.
    pub fn matches_capabilities(&self, capabilities: &Capabilities) -> bool {
        if self.name.to_lowercase() != capabilities.browser_name().to_lowercase() {
            return false;
        }

        if let Some(required_version) = capabilities.browser_version() {
            if !required_version.is_empty() {
                match &self.version {
                    Some(v) => {
                        if v != required_version {
                            return false;
                        }
                    }
                    None => return false,
                }
            }
        }

        if let Some(required_os) = capabilities.platform_name() {
            let required_os = required_os.to_lowercase();
            if required_os.to_lowercase() != "any" {
                match &self.os {
                    Some(os) => {
                        if os.to_lowercase() != required_os {
                            return false;
                        }
                    }
                    None => return false,
                }
            }
        }

        true
    }

    /// Does a preparation of a config for its usage
    /// sets a default fields, make a validation
    pub fn sanitize(&mut self) -> Result<(), XenonError> {
        if self.driver_path.is_none() {
            let default = default_webdriver(&self.name).ok_or_else(|| {
                XenonError::ConfigUnexpectedBrowser(
                    self.name.clone(),
                    "A default webdriver can't be found. You may need to use a custom path option via 'driver_path' setting".to_owned(),
                )
            })?;

            self.driver_path = Some(default.to_owned());
        }

        Ok(())
    }
}

pub fn default_webdriver<S: AsRef<str>>(browser: S) -> Option<&'static Path> {
    match browser.as_ref() {
        "firefox" => Some("geckodriver".as_ref()),
        "chrome" => Some("chromedriver".as_ref()),
        _ => None,
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct W3CCapabilities {
    /// The W3C capabilities object, used to match browser/version/OS etc.
    pub capabilities: serde_json::Value,
    /// All of the additional browser-specific capabilities such as extra arguments etc.
    #[serde(default)]
    pub desired_capabilities: serde_json::Value,
}
