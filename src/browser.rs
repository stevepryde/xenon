use crate::error::XenonError;
use crate::manager::{DriverResolver, DriverVersion};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::info;

pub fn default_sessions_per_driver() -> u32 {
    1
}

pub fn default_max_sessions() -> u32 {
    5
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserConfig {
    name: String,
    version: Option<String>,
    os: Option<String>,
    /// Path to the webdriver binary (e.g. chromedriver, geckodriver).
    /// Populated explicitly in config, or filled in by `sanitize()` from
    /// `default_webdriver()` for known browsers, or by `auto_download` after
    /// resolution. May be `None` for browsers received from a remote node
    /// (which we never spawn ourselves).
    driver_path: Option<PathBuf>,
    args: Option<Vec<String>>,
    #[serde(default = "default_sessions_per_driver")]
    sessions_per_driver: u32,
    #[serde(default = "default_max_sessions")]
    max_sessions: u32,

    /// If `true`, Xenon will download and cache the matching webdriver binary
    /// at startup, replacing `driver_path` with the cached path. Mutually
    /// exclusive with an explicit `driver_path`.
    #[serde(default)]
    #[serde(skip_serializing)]
    auto_download: bool,

    /// Which driver version to use when `auto_download` is enabled. Accepts:
    ///   - `match-local` (default): probe the installed browser and download
    ///     a matching driver
    ///   - `latest`: download the latest stable driver
    ///   - an exact version string (e.g. `"126.0.6478.126"` for Chrome/Edge,
    ///     or `"0.36.0"` for Firefox/geckodriver)
    #[serde(default)]
    #[serde(skip_serializing)]
    driver_version: Option<DriverVersion>,
}

impl BrowserConfig {
    pub fn name(&self) -> &str {
        self.name.as_str()
    }

    /// The driver path. Always `Some` for locally-configured browsers after
    /// `sanitize()` (and, when `auto_download` is set, after `resolve_driver`).
    /// May be `None` for browsers received from a remote node, but those are
    /// never spawned locally.
    pub fn driver_path(&self) -> Option<&Path> {
        self.driver_path.as_deref()
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

    pub fn auto_download(&self) -> bool {
        self.auto_download
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

    /// Validate config values and apply defaults.
    ///
    /// When `auto_download` is set, defer driver-path resolution to
    /// [`Self::resolve_driver`] (which performs network I/O). Otherwise fall
    /// back to the legacy `default_webdriver()` lookup for known browsers.
    pub fn sanitize(&mut self) -> Result<(), XenonError> {
        if self.auto_download {
            if self.driver_path.is_some() {
                return Err(XenonError::ConfigUnexpectedBrowser(
                    self.name.clone(),
                    "auto_download cannot be combined with an explicit driver_path".to_string(),
                ));
            }
            return Ok(());
        }

        if self.driver_path.is_none() {
            let default = default_webdriver(&self.name).ok_or_else(|| {
                XenonError::ConfigUnexpectedBrowser(
                    self.name.clone(),
                    "A default webdriver can't be found. Either set a custom 'driver_path', or enable 'auto_download'".to_owned(),
                )
            })?;

            self.driver_path = Some(default.to_owned());
        }

        Ok(())
    }

    /// If `auto_download` is set, download (or cache-hit) the matching driver
    /// and store its path in `driver_path`. No-op otherwise.
    pub async fn resolve_driver(&mut self, resolver: &DriverResolver) -> Result<(), XenonError> {
        if !self.auto_download {
            return Ok(());
        }
        let version = self.driver_version.clone().unwrap_or_default();
        info!(
            "Resolving webdriver for browser '{}' (version: {:?})...",
            self.name, version
        );
        let path = resolver.resolve(&self.name, &version).await.map_err(|e| {
            XenonError::ConfigUnexpectedBrowser(
                self.name.clone(),
                format!("auto-download failed: {e}"),
            )
        })?;
        info!(
            "Webdriver for browser '{}' resolved to {}",
            self.name,
            path.display()
        );
        self.driver_path = Some(path);
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
