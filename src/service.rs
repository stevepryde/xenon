use crate::browser::{BrowserConfig, Capabilities};
use crate::error::{XenonError, XenonResult};
use crate::portmanager::{PortManager, ServicePort};
use crate::response::XenonResponse;
use crate::session::{Session, XenonSessionId};
use std::collections::HashMap;
use std::path::Path;
use std::process::{Child, Command};
use std::sync::Arc;
use tokio::sync::RwLock;

/// A WebDriverService represents one instance of a webdriver binary such
/// as chromedriver, to which one or more sessions can attach.
#[derive(Debug)]
pub struct WebDriverService {
    port: ServicePort,
    process: Child,
    sessions: HashMap<XenonSessionId, Arc<RwLock<Session>>>,
}

impl WebDriverService {
    pub fn spawn(port: ServicePort, path: &Path) -> XenonResult<Self> {
        let process = Command::new(path)
            .arg("--port")
            .arg(port.to_string())
            .spawn()?;

        Ok(Self {
            port,
            process,
            sessions: HashMap::new(),
        })
    }

    pub fn num_active_sessions(&self) -> usize {
        self.sessions.len()
    }

    pub fn add_session(&mut self, session_id: XenonSessionId, session: Session) {
        self.sessions
            .insert(session_id, Arc::new(RwLock::new(session)));
    }

    pub fn get_session(&self, session_id: &XenonSessionId) -> Option<Arc<RwLock<Session>>> {
        self.sessions.get(session_id).cloned()
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

    pub fn get_service(&self, port: &ServicePort) -> Option<&WebDriverService> {
        self.services.get(&port)
    }

    fn get_next_available_service(
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
            Some(x) => x,
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
            .expect(&format!("No service for port '{}'", next_port)))
    }

    pub fn spawn_session(&mut self, port_manager: &mut PortManager) -> XenonResult<XenonSessionId> {
        let service = self.get_next_available_service(port_manager)?;
        let xsession_id = XenonSessionId::new();
        let session_placeholder = Session::new(service.port);
        service.add_session(xsession_id.clone(), session_placeholder);

        // LOCK services
        // > Sort services by most available slots to fewest available slots
        // > Take first. If no slots available, spawn new.
        // > Lock session slot by inserting a dummy session.
        // UNLOCK services
        // Perform new session request.
        // LOCK sessions
        // If ok, update dummy session with real details and return it.
        // If not ok, clear dummy session and return error.
        // UNLOCK sessions
        Ok(xsession_id)
    }
}
