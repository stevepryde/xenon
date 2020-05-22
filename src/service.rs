use crate::browser::{BrowserConfig, Capabilities};
use crate::error::{XenonError, XenonResult};
use crate::portmanager::{PortManager, ServicePort};
use crate::response::XenonResponse;
use crate::session::XenonSessionId;
use log::*;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::{Child, Command};

/// A WebDriverService represents one instance of a webdriver binary such
/// as chromedriver, to which one or more sessions can attach.
#[derive(Debug)]
pub struct WebDriverService {
    port: ServicePort,
    process: Child,
    sessions: HashSet<XenonSessionId>,
}

impl WebDriverService {
    pub fn spawn(port: ServicePort, path: &Path) -> XenonResult<Self> {
        debug!("Spawning new webdriver on port {}: {:?}", port, path);
        let process = Command::new(path).arg(format!("--port={}", port)).spawn()?;

        Ok(Self {
            port,
            process,
            sessions: HashSet::new(),
        })
    }

    pub fn port(&self) -> ServicePort {
        self.port
    }

    pub fn num_active_sessions(&self) -> usize {
        self.sessions.len()
    }

    pub fn add_session(&mut self, session_id: XenonSessionId) {
        self.sessions.insert(session_id);
    }

    pub fn delete_session(&mut self, session_id: &XenonSessionId) {
        self.sessions.remove(session_id);
    }
}

pub type ServiceGroupName = String;

/// A ServiceGroup represents a provider for a single browser type, which might
/// spawn several instances of the webdriver for that same type. For example
/// geckodriver can only handle a single connection, but a ServiceGroup for
/// geckodriver will spawn a new geckodriver service instance for each new
/// connection, up to the max_sessions limit in the BrowserConfig struct.
#[derive(Debug)]
pub struct ServiceGroup {
    browser: BrowserConfig,
    services: HashMap<ServicePort, WebDriverService>,
}

impl ServiceGroup {
    pub fn new(browser: BrowserConfig) -> Self {
        Self {
            browser,
            services: HashMap::new(),
        }
    }

    pub fn name(&self) -> &str {
        &self.browser.name()
    }

    pub fn matches_capabilities(&self, capabilities: &Capabilities) -> bool {
        self.browser.matches_capabilities(capabilities)
    }

    pub fn total_sessions(&self) -> usize {
        let mut count = 0;
        for service in self.services.values() {
            count += service.num_active_sessions();
        }
        count
    }

    pub fn has_capacity(&self) -> bool {
        let max_sessions = self.browser.max_sessions() as usize;
        self.total_sessions() < max_sessions
    }

    pub fn get_service_mut(&mut self, port: &ServicePort) -> Option<&mut WebDriverService> {
        self.services.get_mut(&port)
    }

    pub fn get_or_start_service(
        &mut self,
        port_manager: &mut PortManager,
    ) -> XenonResult<&mut WebDriverService> {
        let max_per_service = self.browser.sessions_per_driver() as usize;
        let max_sessions = self.browser.max_sessions() as usize;
        let mut session_count = 0;
        let mut next_port: Option<u16> = None;
        let mut best = max_per_service;
        for (k, v) in self.services.iter() {
            let num_sessions = v.sessions.len();
            session_count += num_sessions;
            if session_count >= max_sessions {
                return Err(XenonError::NoSessionsAvailable);
            }
            if session_count < best {
                best = session_count;
                next_port = Some(*k);
            }
        }

        let next_port = match next_port {
            Some(x) => {
                debug!("Matched existing service on port: {}", x);
                x
            }
            None => {
                // Spawn new service.
                let newport = match port_manager.lock_next_port() {
                    Some(p) => p,
                    None => {
                        // We're out of ports.
                        return Err(XenonError::RespondWith(XenonResponse::NoSessionsAvailable));
                    }
                };
                let service = WebDriverService::spawn(newport, &self.browser.driver_path())?;
                self.services.insert(newport, service);
                newport
            }
        };

        // Safe to unwrap here because we literally just either looked it up or inserted it.
        Ok(self
            .services
            .get_mut(&next_port)
            .unwrap_or_else(|| panic!("No service for port '{}'", next_port)))
    }
}
