use std::path::PathBuf;
use std::time::Duration;

use super::browser::BrowserKind;
use super::download::{DownloadConfig, Mirror, ensure_driver, resolve_version};
use super::error::ManagerError;
use super::version::DriverVersion;

const DEFAULT_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(60);

/// Default cache directory: `<cache_dir>/xenon-webdriver/drivers`, falling
/// back to the system temp dir if no cache dir is available.
fn default_cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("xenon-webdriver")
        .join("drivers")
}

/// Resolves a `(browser, version)` pair to a driver binary on disk —
/// downloading and extracting it into a cache if necessary.
///
/// Xenon spawns drivers itself; the resolver only produces the binary path.
#[derive(Debug, Clone)]
pub struct DriverResolver {
    client: reqwest::Client,
    cache_dir: PathBuf,
    download_timeout: Duration,
    offline: bool,
}

impl Default for DriverResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl DriverResolver {
    /// Construct a resolver with default settings.
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .build()
                .expect("default reqwest client should always build"),
            cache_dir: default_cache_dir(),
            download_timeout: DEFAULT_DOWNLOAD_TIMEOUT,
            offline: false,
        }
    }

    /// Resolve a browser name + version spec to a driver binary path.
    ///
    /// Downloads + extracts the matching driver into the cache if it isn't
    /// already there.
    pub async fn resolve(
        &self,
        browser_name: &str,
        version: &DriverVersion,
    ) -> Result<PathBuf, ManagerError> {
        let kind = BrowserKind::from_browser_name(browser_name)?;
        let cfg = DownloadConfig {
            cache_dir: self.cache_dir.clone(),
            mirror: Mirror::default(),
            download_timeout: self.download_timeout,
            offline: self.offline,
        };
        let resolved = resolve_version(&self.client, &cfg, kind, version).await?;
        let driver = ensure_driver(&self.client, &cfg, kind, &resolved).await?;
        Ok(driver.binary)
    }
}
