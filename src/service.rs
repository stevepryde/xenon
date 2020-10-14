use crate::browser::{BrowserConfig, Capabilities};
use crate::error::{XenonError, XenonResult};
use crate::portmanager::{PortManager, ServicePort};
use crate::response::XenonResponse;
use crate::session::XenonSessionId;
use log::*;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use tokio::process::{Child, Command};

/// A WebDriverService represents one instance of a webdriver binary such
/// as chromedriver, to which one or more sessions can attach.
#[derive(Debug)]
pub struct WebDriverService {
    port: ServicePort,
    process: Child,
    sessions: HashSet<XenonSessionId>,
}

impl WebDriverService {
    pub async fn spawn(
        port: ServicePort,
        path: &Path,
        args: &Option<Vec<String>>,
    ) -> XenonResult<Self> {
        let port_arg = &[format!("--port={}", port)];
        let args = args
            .as_ref()
            .map(|args| args.as_slice())
            .unwrap_or_else(|| &[])
            .iter()
            .chain(port_arg);

        debug!(
            "Spawn new WebDriver on port {} with args {:?}: {:?}",
            port, args, path
        );
        let process = Command::new(path).args(args).kill_on_drop(true).spawn()?;
        Ok(Self {
            port,
            process,
            sessions: HashSet::new(),
        })
    }

    pub fn terminate(mut self) {
        assert!(self.sessions.is_empty());

        debug!("Terminate WebDriver on port {}", self.port);
        if let Err(e) = self.process.kill() {
            // What to do? For now just log the error but let everything proceed.
            // TODO: Options:
            //       1. Ignore all such errors indefinitely (but still log them) <-- Current
            //       2. Limp home mode (no new sessions, quit after last session ends, allowing
            //          the service to auto-restart if running in docker etc)
            //       3. Quit if safe - only if session count happens to hit 0 organically
            //       4. Add process to a retry list and keep trying periodically
            error!("Error terminating WebDriver on port {}: {:?}", self.port, e);
        }
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

    pub async fn get_or_start_service(
        &mut self,
        port_manager: &mut PortManager,
    ) -> XenonResult<&mut WebDriverService> {
        let max_per_service = self.browser.sessions_per_driver() as usize;
        let max_sessions = self.browser.max_sessions() as usize;
        let mut overall_session_count = 0;
        let mut next_port: Option<u16> = None;
        let mut best = max_per_service;
        for (k, v) in self.services.iter() {
            let num_sessions_for_service = v.sessions.len();
            overall_session_count += num_sessions_for_service;
            if overall_session_count >= max_sessions {
                return Err(XenonError::NoSessionsAvailable);
            }
            if num_sessions_for_service < best {
                best = num_sessions_for_service;
                next_port = Some(*k);
            }
        }

        let next_port = match next_port {
            Some(p) => p,
            None => {
                // Spawn new service.
                let newport = match port_manager.lock_next_port() {
                    Some(p) => p,
                    None => {
                        // We're all out of ports.
                        return Err(XenonError::RespondWith(XenonResponse::NoSessionsAvailable));
                    }
                };
                let service = WebDriverService::spawn(
                    newport,
                    &self.browser.driver_path(),
                    self.browser.args(),
                )
                .await?;
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

    pub fn delete_session(
        &mut self,
        port: ServicePort,
        xsession_id: &XenonSessionId,
        port_manager: &mut PortManager,
    ) {
        let mut should_terminate = false;
        if let Some(service) = self.services.get_mut(&port) {
            service.delete_session(xsession_id);

            // Should we terminate this session?
            if service.sessions.is_empty() {
                should_terminate = true;
            }
        }

        if should_terminate {
            if let Some(service) = self.services.remove(&port) {
                service.terminate();
                port_manager.unlock_port(port);
            }
        }
    }
}
