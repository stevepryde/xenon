use serde::{Deserialize, Deserializer};

/// How the driver resolver picks which version to download.
///
/// Defaults to [`DriverVersion::MatchLocalBrowser`] — the lowest-friction
/// option for local development.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum DriverVersion {
    /// Probe the locally-installed browser binary for its version, then pick
    /// a matching driver. This is the default.
    #[default]
    MatchLocalBrowser,
    /// Latest stable available from the upstream metadata source.
    Latest,
    /// An exact version string. Chrome / Edge accept either a full version
    /// (`"126.0.6478.126"`) or major-only (`"126"`). Firefox accepts a
    /// geckodriver release tag (e.g. `"0.36.0"`).
    Exact(String),
}

impl<'de> Deserialize<'de> for DriverVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(parse_driver_version(&s))
    }
}

fn parse_driver_version(s: &str) -> DriverVersion {
    let trimmed = s.trim();
    match trimmed.to_ascii_lowercase().as_str() {
        "match-local" | "match_local" | "matchlocalbrowser" | "local" | "current" => {
            DriverVersion::MatchLocalBrowser
        }
        "latest" | "stable" => DriverVersion::Latest,
        _ => DriverVersion::Exact(trimmed.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_keywords() {
        assert_eq!(
            parse_driver_version("match-local"),
            DriverVersion::MatchLocalBrowser
        );
        assert_eq!(
            parse_driver_version("Match-Local"),
            DriverVersion::MatchLocalBrowser
        );
        assert_eq!(
            parse_driver_version("local"),
            DriverVersion::MatchLocalBrowser
        );
        assert_eq!(
            parse_driver_version("current"),
            DriverVersion::MatchLocalBrowser
        );
        assert_eq!(parse_driver_version("LATEST"), DriverVersion::Latest);
    }

    #[test]
    fn parses_exact() {
        assert_eq!(
            parse_driver_version("126.0.6478.126"),
            DriverVersion::Exact("126.0.6478.126".to_string())
        );
        assert_eq!(
            parse_driver_version("0.36.0"),
            DriverVersion::Exact("0.36.0".to_string())
        );
    }
}
