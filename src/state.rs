use crate::browser::Capabilities;
use crate::config::XenonConfig;
use crate::portmanager::{PortManager, ServicePort};
use crate::service::{ServiceGroup, ServiceGroupName};
use crate::session::{Session, XenonSessionId};
use indexmap::map::IndexMap;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

#[derive(Debug)]
pub struct XenonState {
    service_groups: Arc<RwLock<IndexMap<ServiceGroupName, ServiceGroup>>>,
    port_manager: Arc<RwLock<PortManager>>,

    sessions: HashMap<XenonSessionId, Arc<Mutex<Session>>>,
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
            service_groups: Arc::new(RwLock::new(service_groups)),
            port_manager: Arc::new(RwLock::new(port_manager)),
            sessions: HashMap::new(),
        }
    }

    pub fn port_manager(&self) -> Arc<RwLock<PortManager>> {
        self.port_manager.clone()
    }

    pub fn service_groups(&self) -> Arc<RwLock<IndexMap<ServiceGroupName, ServiceGroup>>> {
        self.service_groups.clone()
    }

    pub fn get_session(&self, session_id: &XenonSessionId) -> Option<Arc<Mutex<Session>>> {
        self.sessions.get(session_id).cloned()
    }

    pub fn add_session(&mut self, session_id: XenonSessionId, session: Session) {
        self.sessions
            .insert(session_id, Arc::new(Mutex::new(session)));
    }

    pub fn delete_session(&mut self, session_id: &XenonSessionId) -> bool {
        self.sessions.remove(session_id).is_some()
    }
}
