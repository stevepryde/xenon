use std::path::Path;
use std::process::Command;

use super::error::ManagerError;

/// Browsers the resolver can drive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BrowserKind {
    /// Chrome / Chromium.
    Chrome,
    /// Firefox.
    Firefox,
    /// Microsoft Edge (Chromium-based).
    Edge,
    /// Apple Safari. macOS-only; uses the system `safaridriver` and does not
    /// download anything. Requires `safaridriver --enable` to be run once.
    Safari,
}

impl BrowserKind {
    /// Driver binary name (without `.exe`).
    pub(crate) fn driver_binary_stem(self) -> &'static str {
        match self {
            BrowserKind::Chrome => "chromedriver",
            BrowserKind::Firefox => "geckodriver",
            BrowserKind::Edge => "msedgedriver",
            BrowserKind::Safari => "safaridriver",
        }
    }

    /// Display name used in error hints.
    pub(crate) fn display_name(self) -> &'static str {
        match self {
            BrowserKind::Chrome => "Chrome",
            BrowserKind::Firefox => "Firefox",
            BrowserKind::Edge => "Microsoft Edge",
            BrowserKind::Safari => "Safari",
        }
    }

    /// Cache subdirectory name.
    pub(crate) fn cache_dir_name(self) -> &'static str {
        self.driver_binary_stem()
    }

    /// `true` if the driver is system-managed (no download, no cache).
    pub(crate) fn is_system_managed(self) -> bool {
        matches!(self, BrowserKind::Safari)
    }

    /// Map a `browserName` capability value (case-insensitive) to a
    /// `BrowserKind`. Unknown names yield `UnsupportedBrowser`.
    pub fn from_browser_name(name: &str) -> Result<Self, ManagerError> {
        match name.to_ascii_lowercase().as_str() {
            "chrome" | "chromium" => Ok(BrowserKind::Chrome),
            "firefox" => Ok(BrowserKind::Firefox),
            "microsoftedge" | "msedge" | "edge" => Ok(BrowserKind::Edge),
            "safari" => Ok(BrowserKind::Safari),
            other => Err(ManagerError::UnsupportedBrowser(other.to_string())),
        }
    }
}

/// Probe the locally-installed browser for its version. Tries a list of
/// well-known install locations and returns the first one that responds to
/// `--version` with a parseable version.
pub(crate) fn detect_local_version(browser: BrowserKind) -> Result<String, ManagerError> {
    if browser.is_system_managed() {
        return Ok("system".to_string());
    }

    for candidate in candidate_paths(browser) {
        if let Some(v) = run_version(&candidate, browser) {
            return Ok(v);
        }
    }

    Err(ManagerError::LocalBrowserNotFound {
        browser: browser.display_name(),
        hint: "no installed copy was found; try driver_version: latest or pin an exact version",
    })
}

fn candidate_paths(browser: BrowserKind) -> Vec<String> {
    #[cfg(target_os = "macos")]
    {
        match browser {
            BrowserKind::Chrome => vec![
                "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome".to_string(),
                "/Applications/Chromium.app/Contents/MacOS/Chromium".to_string(),
                "google-chrome".to_string(),
                "chromium".to_string(),
            ],
            BrowserKind::Firefox => vec![
                "/Applications/Firefox.app/Contents/MacOS/firefox".to_string(),
                "firefox".to_string(),
            ],
            BrowserKind::Edge => {
                vec!["/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge".to_string()]
            }
            BrowserKind::Safari => Vec::new(),
        }
    }
    #[cfg(target_os = "linux")]
    {
        match browser {
            BrowserKind::Chrome => vec![
                "google-chrome".to_string(),
                "google-chrome-stable".to_string(),
                "chromium".to_string(),
                "chromium-browser".to_string(),
            ],
            BrowserKind::Firefox => vec!["firefox".to_string(), "firefox-esr".to_string()],
            BrowserKind::Edge => {
                vec![
                    "microsoft-edge".to_string(),
                    "microsoft-edge-stable".to_string(),
                ]
            }
            BrowserKind::Safari => Vec::new(),
        }
    }
    #[cfg(target_os = "windows")]
    {
        match browser {
            BrowserKind::Chrome => vec![
                r"C:\Program Files\Google\Chrome\Application\chrome.exe".to_string(),
                r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe".to_string(),
                "chrome.exe".to_string(),
            ],
            BrowserKind::Firefox => vec![
                r"C:\Program Files\Mozilla Firefox\firefox.exe".to_string(),
                r"C:\Program Files (x86)\Mozilla Firefox\firefox.exe".to_string(),
                "firefox.exe".to_string(),
            ],
            BrowserKind::Edge => vec![
                r"C:\Program Files\Microsoft\Edge\Application\msedge.exe".to_string(),
                r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe".to_string(),
                "msedge.exe".to_string(),
            ],
            BrowserKind::Safari => Vec::new(),
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        let _ = browser;
        Vec::new()
    }
}

fn run_version(path: &str, _browser: BrowserKind) -> Option<String> {
    if !exists_or_in_path(path) {
        return None;
    }

    // On Windows, GUI-subsystem `.exe`s (Chrome, Edge, Firefox) don't reliably
    // write to stdout when invoked from a non-console parent — and msedge.exe
    // can launch the full browser and never return. Read the file's PE
    // VersionInfo via PowerShell instead: same data, no runtime side effects.
    #[cfg(target_os = "windows")]
    {
        return read_pe_version(path);
    }

    #[cfg(not(target_os = "windows"))]
    {
        let output = Command::new(path).arg("--version").output().ok()?;
        if !output.status.success() {
            return None;
        }
        parse_version(&String::from_utf8_lossy(&output.stdout))
    }
}

#[cfg(target_os = "windows")]
fn read_pe_version(path: &str) -> Option<String> {
    let abs = if Path::new(path).is_absolute() {
        path.to_string()
    } else {
        let out = Command::new("where").arg(path).output().ok()?;
        if !out.status.success() {
            return None;
        }
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .next()?
            .trim()
            .to_string()
    };
    let escaped = abs.replace('\'', "''");
    let script = format!("(Get-Item -LiteralPath '{escaped}').VersionInfo.ProductVersion");
    let output = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_version(&String::from_utf8_lossy(&output.stdout))
}

fn exists_or_in_path(path: &str) -> bool {
    let p = Path::new(path);
    if p.is_absolute() {
        return p.exists();
    }
    // Bare command name — let the spawn attempt decide; PATH lookup is the OS's
    // job.
    true
}

/// Extract a version like `126.0.6478.126` (or `126.0`) from arbitrary
/// `--version` output.
pub(crate) fn parse_version(s: &str) -> Option<String> {
    let mut it = s.chars().peekable();
    while it.peek().is_some() {
        if !it.peek().is_some_and(|c| c.is_ascii_digit()) {
            it.next();
            continue;
        }
        let mut buf = String::new();
        while let Some(&c) = it.peek() {
            if c.is_ascii_digit() || c == '.' {
                buf.push(c);
                it.next();
            } else {
                break;
            }
        }
        if buf.contains('.') && buf.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            let trimmed = buf.trim_end_matches('.');
            return Some(trimmed.to_string());
        }
    }
    None
}

/// Major version (the segment before the first dot).
pub(crate) fn major(version: &str) -> &str {
    version.split('.').next().unwrap_or(version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_chrome_version() {
        assert_eq!(
            parse_version("Google Chrome 126.0.6478.126 \n").as_deref(),
            Some("126.0.6478.126")
        );
    }

    #[test]
    fn parse_firefox_version() {
        assert_eq!(
            parse_version("Mozilla Firefox 128.0.2").as_deref(),
            Some("128.0.2")
        );
    }

    #[test]
    fn major_strips() {
        assert_eq!(major("126.0.6478.126"), "126");
        assert_eq!(major("128"), "128");
    }

    #[test]
    fn from_browser_name_aliases() {
        assert_eq!(
            BrowserKind::from_browser_name("chrome").unwrap(),
            BrowserKind::Chrome
        );
        assert_eq!(
            BrowserKind::from_browser_name("Chromium").unwrap(),
            BrowserKind::Chrome
        );
        assert_eq!(
            BrowserKind::from_browser_name("firefox").unwrap(),
            BrowserKind::Firefox
        );
        assert_eq!(
            BrowserKind::from_browser_name("MicrosoftEdge").unwrap(),
            BrowserKind::Edge
        );
        assert_eq!(
            BrowserKind::from_browser_name("msedge").unwrap(),
            BrowserKind::Edge
        );
        assert_eq!(
            BrowserKind::from_browser_name("safari").unwrap(),
            BrowserKind::Safari
        );
        assert!(matches!(
            BrowserKind::from_browser_name("ie"),
            Err(ManagerError::UnsupportedBrowser(_))
        ));
    }

    #[test]
    fn safari_is_system_managed() {
        assert!(BrowserKind::Safari.is_system_managed());
        assert!(!BrowserKind::Chrome.is_system_managed());
    }
}
