use crate::browser::BrowserConfig;
use crate::error::XenonError;
use crate::portmanager::ServicePort;
use log::*;
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Default, Deserialize)]
pub struct XenonConfig {
    browsers: Vec<BrowserConfig>,
    ports: Vec<String>,
}

impl XenonConfig {
    pub fn new() -> Self {
        // TODO: validate config.
        Self::default()
    }

    /// Get the port list as a Vec of individual ports.
    pub fn get_port_list(&self) -> Vec<ServicePort> {
        let port_list = parse_port_list(&self.ports);
        let max_sessions =
            self.browsers
                .iter()
                .fold(0, |acc, browser| acc + browser.max_sessions()) as usize;
        if port_list.len() < max_sessions {
            warn!(
                "Number of ports ({}) is less than the maximum number of sessions ({})",
                port_list.len(),
                max_sessions
            );
        }
        port_list
    }

    /// Get the list of browsers and consume the config.
    pub fn browsers(self) -> Vec<BrowserConfig> {
        self.browsers
    }
}

pub fn load_config(config_path: &Path) -> Result<XenonConfig, XenonError> {
    if !config_path.exists() {
        return Err(XenonError::ConfigNotFound(config_path.to_path_buf()));
    }

    let config_str = std::fs::read_to_string(config_path)
        .map_err(|e| XenonError::ConfigLoadError(config_path.to_path_buf(), e.to_string()))?;
    let mut config: XenonConfig = serde_yaml::from_str(&config_str)
        .map_err(|e| XenonError::ConfigLoadError(config_path.to_path_buf(), e.to_string()))?;

    for browser_cfg in &mut config.browsers {
        browser_cfg.sanitize()?;
    }

    Ok(config)
}

pub fn parse_port_list<T: AsRef<str>>(port_ranges: &[T]) -> Vec<ServicePort> {
    let mut ports = Vec::new();

    for port_range in port_ranges {
        let range = port_range.as_ref();
        let parts: Vec<&str> = range.splitn(2, '-').collect();
        match parts.len() {
            1 => match parts[0].parse::<ServicePort>() {
                Ok(x) => ports.push(x),
                Err(e) => {
                    error!("Invalid port '{}': {}", range, e.to_string());
                }
            },
            2 => {
                let start: ServicePort = match parts[0].parse() {
                    Ok(x) => x,
                    Err(e) => {
                        error!(
                            "Invalid port {} in port range '{}': {}",
                            parts[0],
                            range,
                            e.to_string()
                        );
                        continue;
                    }
                };
                let end: ServicePort = match parts[1].parse() {
                    Ok(x) => x,
                    Err(e) => {
                        error!(
                            "Invalid port {} in port range '{}': {}",
                            parts[1],
                            range,
                            e.to_string()
                        );
                        continue;
                    }
                };
                if start <= 1024 || end <= 1024 {
                    error!("Only ports > 1024 are allowed");
                    continue;
                }
                if end < start {
                    error!("Start port must precede end port");
                    continue;
                }
                for p in start..=end {
                    ports.push(p);
                }
            }
            _ => unreachable!(),
        }
    }

    ports
}

#[cfg(test)]
mod test {
    use crate::config::parse_port_list;

    #[test]
    fn test_port_parser_empty() {
        let empty_vec: Vec<u16> = Vec::new();
        let empty_input_vec: Vec<String> = Vec::new();
        assert_eq!(parse_port_list(&empty_input_vec), empty_vec);
    }

    #[test]
    fn test_port_parser_single() {
        assert_eq!(parse_port_list(&["2000"]), vec![2000]);
    }

    #[test]
    fn test_port_parser_range() {
        assert_eq!(parse_port_list(&["2000-2001"]), vec![2000, 2001]);
        assert_eq!(parse_port_list(&["2000-2000"]), vec![2000]);

        // Errors are logged but ignored.
        let empty_vec: Vec<u16> = Vec::new();
        assert_eq!(parse_port_list(&["1000-2000"]), empty_vec);
        assert_eq!(parse_port_list(&["2000-3000-4000"]), empty_vec);
        assert_eq!(parse_port_list(&["2000-2001", "adfasd"]), vec![2000, 2001]);
    }
}
