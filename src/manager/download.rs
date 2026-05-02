use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Deserialize;
use tracing::{debug, info, warn};
use url::Url;

use super::browser::{BrowserKind, major};
use super::error::ManagerError;
use super::version::DriverVersion;

/// Built-in upstream metadata sources.
#[derive(Debug, Clone)]
pub(crate) struct Mirror {
    pub chrome_metadata: Url,
    pub geckodriver_downloads: Url,
    pub edge_downloads: Url,
}

impl Default for Mirror {
    fn default() -> Self {
        Self {
            chrome_metadata: Url::parse("https://googlechromelabs.github.io/").unwrap(),
            geckodriver_downloads: Url::parse(
                "https://github.com/mozilla/geckodriver/releases/download/",
            )
            .unwrap(),
            edge_downloads: Url::parse("https://msedgedriver.microsoft.com/").unwrap(),
        }
    }
}

/// Configuration values consumed by the download / cache logic.
pub(crate) struct DownloadConfig {
    pub cache_dir: PathBuf,
    pub mirror: Mirror,
    pub download_timeout: Duration,
    pub offline: bool,
}

/// Resolve [`DriverVersion`] to a concrete version string.
pub(crate) async fn resolve_version(
    client: &reqwest::Client,
    cfg: &DownloadConfig,
    browser: BrowserKind,
    spec: &DriverVersion,
) -> Result<String, ManagerError> {
    if browser == BrowserKind::Safari {
        return Ok("system".to_string());
    }

    debug!(?spec, ?browser, "resolving driver version");

    let resolve_firefox_for_browser_version = |fx: &str| -> Result<String, ManagerError> {
        geckodriver_for_firefox(fx)
            .map(str::to_owned)
            .ok_or_else(|| {
                ManagerError::Parse(format!(
                    "no geckodriver release in compatibility table covers Firefox {fx}"
                ))
            })
    };

    let resolved = match spec {
        DriverVersion::Exact(v) => match browser {
            BrowserKind::Chrome => resolve_chrome_exact(client, cfg, v).await?,
            BrowserKind::Firefox => resolve_firefox_exact(v),
            BrowserKind::Edge => resolve_edge_exact(client, cfg, v).await?,
            BrowserKind::Safari => unreachable!("safari short-circuited above"),
        },
        DriverVersion::Latest => match browser {
            BrowserKind::Chrome => fetch_chrome_latest(client, cfg).await?,
            BrowserKind::Firefox => firefox_latest_from_table().to_owned(),
            BrowserKind::Edge => fetch_edge_latest(client, cfg).await?,
            BrowserKind::Safari => unreachable!("safari short-circuited above"),
        },
        DriverVersion::MatchLocalBrowser => {
            let v = super::browser::detect_local_version(browser)?;
            match browser {
                BrowserKind::Chrome => resolve_chrome_exact(client, cfg, &v).await?,
                BrowserKind::Edge => resolve_edge_exact(client, cfg, &v).await?,
                BrowserKind::Firefox => resolve_firefox_for_browser_version(&v)?,
                BrowserKind::Safari => unreachable!("safari short-circuited above"),
            }
        }
    };

    info!(%resolved, ?browser, "driver version resolved");
    Ok(resolved)
}

/// One entry in the geckodriver↔Firefox compatibility table.
struct GeckodriverRelease {
    version: &'static str,
    min_firefox: u32,
    max_firefox: Option<u32>,
}

/// Embedded geckodriver compatibility table, sorted descending by geckodriver
/// version. Mirrors SeleniumHQ's `geckodriver-support.json`.
const GECKODRIVER_RELEASES: &[GeckodriverRelease] = &[
    GeckodriverRelease {
        version: "0.36.0",
        min_firefox: 128,
        max_firefox: None,
    },
    GeckodriverRelease {
        version: "0.35.0",
        min_firefox: 115,
        max_firefox: None,
    },
    GeckodriverRelease {
        version: "0.34.0",
        min_firefox: 115,
        max_firefox: None,
    },
    GeckodriverRelease {
        version: "0.33.0",
        min_firefox: 102,
        max_firefox: Some(120),
    },
    GeckodriverRelease {
        version: "0.32.2",
        min_firefox: 102,
        max_firefox: Some(120),
    },
    GeckodriverRelease {
        version: "0.31.0",
        min_firefox: 91,
        max_firefox: Some(120),
    },
    GeckodriverRelease {
        version: "0.30.0",
        min_firefox: 78,
        max_firefox: Some(90),
    },
    GeckodriverRelease {
        version: "0.29.1",
        min_firefox: 60,
        max_firefox: Some(90),
    },
];

fn geckodriver_for_firefox(firefox_version: &str) -> Option<&'static str> {
    let major: u32 = firefox_version
        .split('.')
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    GECKODRIVER_RELEASES
        .iter()
        .find(|r| major >= r.min_firefox && r.max_firefox.is_none_or(|max| major <= max))
        .map(|r| r.version)
}

#[derive(Deserialize)]
struct ChromeKnownGoodVersions {
    versions: Vec<ChromeVersionEntry>,
}

#[derive(Deserialize)]
struct ChromeVersionEntry {
    version: String,
    downloads: ChromeDownloads,
}

#[derive(Deserialize)]
struct ChromeDownloads {
    #[serde(default)]
    chromedriver: Vec<ChromePlatformDownload>,
}

#[derive(Deserialize, Clone)]
struct ChromePlatformDownload {
    platform: String,
    url: String,
}

async fn fetch_chrome_index(
    client: &reqwest::Client,
    cfg: &DownloadConfig,
) -> Result<ChromeKnownGoodVersions, ManagerError> {
    let url = cfg
        .mirror
        .chrome_metadata
        .join("chrome-for-testing/known-good-versions-with-downloads.json")
        .map_err(|e| ManagerError::Parse(e.to_string()))?;
    let resp = client
        .get(url)
        .timeout(cfg.download_timeout)
        .send()
        .await?
        .error_for_status()?;
    let body: ChromeKnownGoodVersions = resp.json().await?;
    Ok(body)
}

async fn fetch_chrome_latest(
    client: &reqwest::Client,
    cfg: &DownloadConfig,
) -> Result<String, ManagerError> {
    let url = cfg
        .mirror
        .chrome_metadata
        .join("chrome-for-testing/LATEST_RELEASE_STABLE")
        .map_err(|e| ManagerError::Parse(e.to_string()))?;
    let resp = client
        .get(url)
        .timeout(cfg.download_timeout)
        .send()
        .await?
        .error_for_status()?;
    Ok(resp.text().await?.trim().to_string())
}

async fn resolve_chrome_exact(
    client: &reqwest::Client,
    cfg: &DownloadConfig,
    version: &str,
) -> Result<String, ManagerError> {
    let index = fetch_chrome_index(client, cfg).await?;
    if version.contains('.') && index.versions.iter().any(|v| v.version == version) {
        return Ok(version.to_string());
    }
    let m = major(version);
    let mut matches: Vec<&ChromeVersionEntry> = index
        .versions
        .iter()
        .filter(|v| major(&v.version) == m)
        .collect();
    matches.sort_by(|a, b| sort_semver(&a.version, &b.version));
    matches
        .last()
        .map(|v| v.version.clone())
        .ok_or_else(|| ManagerError::Parse(format!("no chromedriver release for major {m}")))
}

fn sort_semver(a: &str, b: &str) -> std::cmp::Ordering {
    let parse = |s: &str| -> Vec<u32> {
        s.split('.')
            .map(|p| p.parse::<u32>().unwrap_or(0))
            .collect()
    };
    parse(a).cmp(&parse(b))
}

fn firefox_latest_from_table() -> &'static str {
    GECKODRIVER_RELEASES
        .first()
        .expect("GECKODRIVER_RELEASES must not be empty")
        .version
}

async fn fetch_edge_latest(
    client: &reqwest::Client,
    cfg: &DownloadConfig,
) -> Result<String, ManagerError> {
    let url = cfg
        .mirror
        .edge_downloads
        .join("LATEST_STABLE")
        .map_err(|e| ManagerError::Parse(e.to_string()))?;
    let bytes = client
        .get(url)
        .timeout(cfg.download_timeout)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    Ok(decode_edge_text(&bytes).trim().to_string())
}

async fn resolve_edge_exact(
    client: &reqwest::Client,
    cfg: &DownloadConfig,
    version: &str,
) -> Result<String, ManagerError> {
    if version.contains('.') {
        return Ok(version.to_string());
    }
    let url = cfg
        .mirror
        .edge_downloads
        .join(&format!("LATEST_RELEASE_{version}"))
        .map_err(|e| ManagerError::Parse(e.to_string()))?;
    let resp = client.get(url).timeout(cfg.download_timeout).send().await?;
    if !resp.status().is_success() {
        return fetch_edge_latest(client, cfg).await;
    }
    let bytes = resp.bytes().await?;
    Ok(decode_edge_text(&bytes).trim().to_string())
}

/// The msedgedriver `LATEST_*` endpoints return text encoded as UTF-16 LE with
/// a BOM. Decode that, falling back to UTF-8 if no BOM is present.
fn decode_edge_text(bytes: &[u8]) -> String {
    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        let utf16: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16_lossy(&utf16)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    }
}

fn resolve_firefox_exact(version: &str) -> String {
    // Treat as a literal geckodriver tag — `0.36.0` etc.
    version.trim_start_matches('v').to_string()
}

/// Where a downloaded driver binary lives in the cache.
pub(crate) struct DriverPath {
    pub binary: PathBuf,
}

/// Ensure the driver for `(browser, version)` is present in the cache;
/// download it if not. Returns the path to the executable.
pub(crate) async fn ensure_driver(
    client: &reqwest::Client,
    cfg: &DownloadConfig,
    browser: BrowserKind,
    version: &str,
) -> Result<DriverPath, ManagerError> {
    if browser == BrowserKind::Safari {
        return safari_driver_path();
    }

    let platform = match browser {
        BrowserKind::Chrome | BrowserKind::Firefox => chrome_platform(),
        BrowserKind::Edge => edge_platform(),
        BrowserKind::Safari => unreachable!("safari short-circuited above"),
    };
    let dir = cfg
        .cache_dir
        .join(browser.cache_dir_name())
        .join(version)
        .join(platform);
    let bin_name = exe_name(browser);
    let bin_path = dir.join(&bin_name);

    if bin_path.exists() {
        debug!(path = %bin_path.display(), "driver cache hit");
        return Ok(DriverPath { binary: bin_path });
    }

    if cfg.offline {
        return Err(ManagerError::Offline(bin_path));
    }

    tokio::fs::create_dir_all(&dir).await?;
    download_and_extract(client, cfg, browser, version, &dir).await?;
    info!(path = %bin_path.display(), "driver archive extracted");

    #[cfg(unix)]
    if bin_path.exists() {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = tokio::fs::metadata(&bin_path).await?.permissions();
        perms.set_mode(0o755);
        tokio::fs::set_permissions(&bin_path, perms).await?;
    }

    Ok(DriverPath { binary: bin_path })
}

/// Locate the system-installed `safaridriver`. Currently only macOS ships one.
fn safari_driver_path() -> Result<DriverPath, ManagerError> {
    #[cfg(target_os = "macos")]
    {
        let p = Path::new("/usr/bin/safaridriver");
        if p.exists() {
            return Ok(DriverPath {
                binary: p.to_path_buf(),
            });
        }
        Err(ManagerError::LocalBrowserNotFound {
            browser: "Safari",
            hint: "/usr/bin/safaridriver not found; install Safari and run `safaridriver --enable`",
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err(ManagerError::UnsupportedBrowser(
            "safari (only available on macOS)".to_string(),
        ))
    }
}

fn exe_name(browser: BrowserKind) -> String {
    let stem = browser.driver_binary_stem();
    if cfg!(windows) {
        format!("{stem}.exe")
    } else {
        stem.to_string()
    }
}

async fn download_and_extract(
    client: &reqwest::Client,
    cfg: &DownloadConfig,
    browser: BrowserKind,
    version: &str,
    target_dir: &Path,
) -> Result<(), ManagerError> {
    match browser {
        BrowserKind::Chrome => download_chromedriver(client, cfg, version, target_dir).await,
        BrowserKind::Firefox => download_geckodriver(client, cfg, version, target_dir).await,
        BrowserKind::Edge => download_msedgedriver(client, cfg, version, target_dir).await,
        BrowserKind::Safari => Err(ManagerError::Download(
            "Safari is system-managed; ensure_driver should not have reached download path".into(),
        )),
    }
}

async fn download_msedgedriver(
    client: &reqwest::Client,
    cfg: &DownloadConfig,
    version: &str,
    target_dir: &Path,
) -> Result<(), ManagerError> {
    let platform = edge_platform();
    let url = cfg
        .mirror
        .edge_downloads
        .join(&format!("{version}/edgedriver_{platform}.zip"))
        .map_err(|e| ManagerError::Parse(e.to_string()))?;
    let bytes = fetch_bytes_with_retry(
        client,
        &url,
        cfg.download_timeout,
        BrowserKind::Edge,
        version,
    )
    .await?;
    extract_zip(&bytes, target_dir, BrowserKind::Edge)
}

async fn download_chromedriver(
    client: &reqwest::Client,
    cfg: &DownloadConfig,
    version: &str,
    target_dir: &Path,
) -> Result<(), ManagerError> {
    let index = fetch_chrome_index(client, cfg).await?;
    let entry = index
        .versions
        .iter()
        .find(|v| v.version == version)
        .ok_or_else(|| {
            ManagerError::Parse(format!("chromedriver version {version} not in CfT index"))
        })?;
    let platform = chrome_platform();
    let download = entry
        .downloads
        .chromedriver
        .iter()
        .find(|d| d.platform == platform)
        .ok_or_else(|| {
            ManagerError::Parse(format!(
                "chromedriver {version} has no download for platform {platform}"
            ))
        })?;
    let url = download
        .url
        .parse::<Url>()
        .map_err(|e| ManagerError::Parse(format!("invalid chromedriver download URL: {e}")))?;
    let bytes = fetch_bytes_with_retry(
        client,
        &url,
        cfg.download_timeout,
        BrowserKind::Chrome,
        version,
    )
    .await?;
    extract_zip(&bytes, target_dir, BrowserKind::Chrome)
}

/// GET a URL into bytes, retrying on transient failures (5xx and network
/// errors). 4xx errors surface immediately.
async fn fetch_bytes_with_retry(
    client: &reqwest::Client,
    url: &Url,
    timeout: Duration,
    browser: BrowserKind,
    version: &str,
) -> Result<bytes::Bytes, ManagerError> {
    const MAX_ATTEMPTS: u32 = 3;
    let mut last_err: Option<ManagerError> = None;
    let started = Instant::now();
    info!(%url, ?browser, %version, "driver download started");
    for attempt in 0..MAX_ATTEMPTS {
        let result: Result<bytes::Bytes, ManagerError> = async {
            let resp = client
                .get(url.clone())
                .header("User-Agent", "xenon-webdriver-manager")
                .timeout(timeout)
                .send()
                .await?;
            let status = resp.status();
            if status.is_client_error() {
                return Err(ManagerError::Http(format!("HTTP {status} for {url}")));
            }
            if !status.is_success() {
                return Err(ManagerError::Http(format!("HTTP {status} for {url}")));
            }
            Ok(resp.bytes().await?)
        }
        .await;

        match result {
            Ok(b) => {
                info!(
                    bytes = b.len(),
                    duration_ms = started.elapsed().as_millis() as u64,
                    "driver download complete"
                );
                return Ok(b);
            }
            Err(e @ ManagerError::Http(_)) if is_transient(&e) => {
                warn!(attempt = attempt + 1, error = %e, "driver download retry");
                last_err = Some(e);
            }
            Err(e) => return Err(e),
        }

        if attempt + 1 < MAX_ATTEMPTS {
            let backoff = Duration::from_secs(1u64 << attempt);
            tokio::time::sleep(backoff).await;
        }
    }
    Err(last_err.unwrap_or_else(|| ManagerError::Http("retry budget exhausted".into())))
}

fn is_transient(err: &ManagerError) -> bool {
    let ManagerError::Http(msg) = err else {
        return false;
    };
    msg.contains("HTTP 5")
        || msg.contains("operation timed out")
        || msg.contains("connection")
        || msg.contains("dns error")
        || msg.contains("error sending request")
}

/// Extract the driver binary for `browser` from a ZIP archive, writing it to
/// `target_dir`.
pub(crate) fn extract_zip(
    bytes: &[u8],
    target_dir: &Path,
    browser: BrowserKind,
) -> Result<(), ManagerError> {
    let cursor = std::io::Cursor::new(bytes);
    let mut zip = zip::ZipArchive::new(cursor).map_err(|e| ManagerError::Extract(e.to_string()))?;
    let exe = exe_name(browser);
    for i in 0..zip.len() {
        let mut entry = zip
            .by_index(i)
            .map_err(|e| ManagerError::Extract(e.to_string()))?;
        let entry_name = entry.name().to_string();
        let basename = Path::new(&entry_name)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        if basename == exe {
            let mut out = std::fs::File::create(target_dir.join(&exe)).map_err(ManagerError::Io)?;
            std::io::copy(&mut entry, &mut out)
                .map_err(|e| ManagerError::Extract(e.to_string()))?;
            return Ok(());
        }
    }
    Err(ManagerError::Extract(format!(
        "{exe} not found inside {} archive",
        browser.cache_dir_name()
    )))
}

async fn download_geckodriver(
    client: &reqwest::Client,
    cfg: &DownloadConfig,
    version: &str,
    target_dir: &Path,
) -> Result<(), ManagerError> {
    let url = geckodriver_download_url(cfg, version)?;
    let bytes = fetch_bytes_with_retry(
        client,
        &url,
        cfg.download_timeout,
        BrowserKind::Firefox,
        version,
    )
    .await?;
    if cfg!(windows) {
        extract_zip(&bytes, target_dir, BrowserKind::Firefox)
    } else {
        extract_geckodriver_tar_gz(&bytes, target_dir)
    }
}

fn geckodriver_download_url(cfg: &DownloadConfig, version: &str) -> Result<Url, ManagerError> {
    let v = version.trim_start_matches('v');
    let asset = geckodriver_asset_name(v);
    cfg.mirror
        .geckodriver_downloads
        .join(&format!("v{v}/{asset}"))
        .map_err(|e| ManagerError::Parse(e.to_string()))
}

fn geckodriver_asset_name(version: &str) -> String {
    let suffix = if cfg!(target_os = "windows") {
        if cfg!(target_arch = "aarch64") {
            "win-aarch64.zip"
        } else if cfg!(target_pointer_width = "64") {
            "win64.zip"
        } else {
            "win32.zip"
        }
    } else if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") {
            "macos-aarch64.tar.gz"
        } else {
            "macos.tar.gz"
        }
    } else if cfg!(target_arch = "aarch64") {
        "linux-aarch64.tar.gz"
    } else {
        "linux64.tar.gz"
    };
    format!("geckodriver-v{version}-{suffix}")
}

fn extract_geckodriver_tar_gz(bytes: &[u8], target_dir: &Path) -> Result<(), ManagerError> {
    let gz = flate2::read::GzDecoder::new(std::io::Cursor::new(bytes));
    let mut archive = tar::Archive::new(gz);
    let exe = exe_name(BrowserKind::Firefox);
    for entry in archive
        .entries()
        .map_err(|e| ManagerError::Extract(e.to_string()))?
    {
        let mut entry = entry.map_err(|e| ManagerError::Extract(e.to_string()))?;
        let path = entry
            .path()
            .map_err(|e| ManagerError::Extract(e.to_string()))?;
        let basename = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        if basename == exe {
            let out_path = target_dir.join(&exe);
            entry
                .unpack(&out_path)
                .map_err(|e| ManagerError::Extract(e.to_string()))?;
            return Ok(());
        }
    }
    Err(ManagerError::Extract(format!(
        "{exe} not found inside geckodriver archive"
    )))
}

fn chrome_platform() -> &'static str {
    if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") {
            "mac-arm64"
        } else {
            "mac-x64"
        }
    } else if cfg!(target_os = "windows") {
        if cfg!(target_pointer_width = "64") {
            "win64"
        } else {
            "win32"
        }
    } else {
        "linux64"
    }
}

fn edge_platform() -> &'static str {
    if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") {
            "mac64_m1"
        } else {
            "mac64"
        }
    } else if cfg!(target_os = "windows") {
        if cfg!(target_arch = "aarch64") {
            "arm64"
        } else if cfg!(target_pointer_width = "64") {
            "win64"
        } else {
            "win32"
        }
    } else {
        "linux64"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn semver_sort() {
        assert_eq!(
            sort_semver("126.0.6478.10", "126.0.6478.126"),
            std::cmp::Ordering::Less
        );
        assert_eq!(
            sort_semver("127.0.0.0", "126.99.99.99"),
            std::cmp::Ordering::Greater
        );
    }

    #[test]
    fn chrome_platform_known() {
        let p = chrome_platform();
        assert!(["mac-arm64", "mac-x64", "win64", "win32", "linux64"].contains(&p));
    }

    #[test]
    fn edge_platform_known() {
        let p = edge_platform();
        assert!(["mac64_m1", "mac64", "win64", "win32", "arm64", "linux64"].contains(&p));
    }

    #[test]
    fn decode_edge_text_utf16_bom() {
        let bytes = [0xFF, 0xFE, 0x31, 0x00, 0x32, 0x00, 0x36, 0x00];
        assert_eq!(decode_edge_text(&bytes).trim(), "126");
    }

    #[test]
    fn decode_edge_text_utf8_fallback() {
        let bytes = b"126.0.6478.126\n";
        assert_eq!(decode_edge_text(bytes).trim(), "126.0.6478.126");
    }

    #[test]
    fn extract_zip_finds_chromedriver() {
        let exe = exe_name(BrowserKind::Chrome);
        let inner_path = format!("chromedriver-linux64/{exe}");

        let mut buf = Vec::new();
        {
            let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            zip.start_file::<_, ()>(&inner_path, Default::default())
                .unwrap();
            zip.write_all(b"#!/bin/sh\necho fake driver\n").unwrap();
            zip.finish().unwrap();
        }

        let dir = tempfile::tempdir().unwrap();
        extract_zip(&buf, dir.path(), BrowserKind::Chrome).unwrap();

        let extracted = dir.path().join(&exe);
        assert!(extracted.exists());
    }

    #[test]
    fn geckodriver_for_firefox_table() {
        assert_eq!(geckodriver_for_firefox("150.0"), Some("0.36.0"));
        assert_eq!(geckodriver_for_firefox("128.0.1"), Some("0.36.0"));
        assert_eq!(geckodriver_for_firefox("127.0"), Some("0.35.0"));
        assert_eq!(geckodriver_for_firefox("114.0"), Some("0.33.0"));
        assert_eq!(geckodriver_for_firefox("91.0"), Some("0.31.0"));
        assert_eq!(geckodriver_for_firefox("80.0"), Some("0.30.0"));
        assert_eq!(geckodriver_for_firefox("60.0"), Some("0.29.1"));
        assert_eq!(geckodriver_for_firefox("50.0"), None);
    }

    #[test]
    fn firefox_latest_is_table_head() {
        assert_eq!(firefox_latest_from_table(), "0.36.0");
    }

    #[test]
    fn transient_error_classification() {
        assert!(is_transient(&ManagerError::Http(
            "HTTP 502 Bad Gateway for ...".into()
        )));
        assert!(is_transient(&ManagerError::Http(
            "error sending request".into()
        )));
        assert!(!is_transient(&ManagerError::Http(
            "HTTP 404 Not Found".into()
        )));
        assert!(!is_transient(&ManagerError::Parse("bad version".into())));
    }
}
