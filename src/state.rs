use crate::browser::Capabilities;
use crate::config::XenonConfig;
use crate::portmanager::{PortManager, ServicePort};
use crate::service::{ServiceGroup, ServiceGroupName};
use crate::session::{Session, XenonSessionId};
use indexmap::map::IndexMap;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug)]
pub struct XenonState {
    service_groups: IndexMap<ServiceGroupName, ServiceGroup>,
    port_manager: Arc<RwLock<PortManager>>,
    sessions: HashMap<XenonSessionId, (ServiceGroupName, ServicePort)>,
}

impl XenonState {
    pub fn new(config: XenonConfig) -> Self {
        let port_manager = PortManager::new(&config);
        let mut service_groups = IndexMap::new();
        for browser in config.browsers() {
            let group = ServiceGroup::new(browser);
            service_groups.insert(group.name().to_string(), group);
        }

        Self {
            service_groups,
            port_manager: Arc::new(RwLock::new(port_manager)),
            sessions: HashMap::new(),
        }
    }

    pub fn match_capabilities(&self, capabilities: &Capabilities) -> Option<&ServiceGroup> {
        for group in self.service_groups.values() {
            if group.matches_capabilities(capabilities) {
                return Some(group);
            }
        }
        None
    }

    pub fn port_manager(&self) -> Arc<RwLock<PortManager>> {
        self.port_manager.clone()
    }

    pub fn service_group(&self, name: &str) -> Option<&ServiceGroup> {
        self.service_groups.get(name)
    }

    pub fn service_group_mut(&mut self, name: &str) -> Option<&mut ServiceGroup> {
        self.service_groups.get_mut(name)
    }

    pub fn get_session(&self, session_id: &XenonSessionId) -> Option<Arc<RwLock<Session>>> {
        let (group_name, port) = self.sessions.get(session_id)?;
        let group = self.service_group(group_name)?;
        let service = group.get_service(port)?;
        service.get_session(session_id)
    }

    pub fn add_session_index(
        &mut self,
        session_id: XenonSessionId,
        service_group_name: ServiceGroupName,
        service_port: ServicePort,
    ) {
        self.sessions
            .insert(session_id, (service_group_name, service_port));
    }

    pub fn delete_session_index(&mut self, session_id: &XenonSessionId) -> bool {
        self.sessions.remove(session_id).is_some()
    }
}
