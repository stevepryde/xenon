use crate::config::XenonConfig;
use crate::error::XenonResult;
use crate::nodes::{NodeId, RemoteNode};
use crate::portmanager::PortManager;
use crate::service::{ServiceGroup, ServiceGroupName};
use crate::session::{Session, XenonSessionId};
use indexmap::map::IndexMap;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

#[derive(Debug)]
pub struct XenonState {
    // The service groups and port manager are each wrapped in Arc so that they
    // can be used outside of state. They are also wrapped in RwLock because
    // the majority of uses will be reads but when either creating or removing
    // a service we will need to write. They are wrapped in Arc independently
    // because we only need the port manager when spawning or terminating a
    // service, and since some services can support multiple sessions we may
    // want the ability to spawn / terminate a session without requiring a
    // write-lock on the port manager.
    service_groups: Arc<RwLock<IndexMap<ServiceGroupName, ServiceGroup>>>,
    port_manager: Arc<RwLock<PortManager>>,

    // Each individual session is wrapped in Arc so that it can be used outside of state.
    // It is also wrapped in Mutex so that each session can only make 1 request at a time.
    // Separate sessions can still make requests in parallel however.
    // The sessions are kept separate from service groups because we want to keep the
    // main session path lock-free where we are simply using a session and not
    // creating or deleting one.
    sessions: HashMap<XenonSessionId, Arc<Mutex<Session>>>,

    // A RemoteNode can contain several RemoteServiceGroup entries. Each of these
    // behaves similarly to a local ServiceGroup, but the requests are forwarded
    // to its parent node instead of a local service.
    // When matching capabilities, local service groups are always preferred.
    // Remote nodes will be queried only when local service groups cannot service
    // a new session request.
    remote_nodes: Arc<RwLock<IndexMap<NodeId, RemoteNode>>>,
}

impl XenonState {
    pub fn new(config: XenonConfig) -> XenonResult<Self> {
        let port_manager = PortManager::new(&config);
        let mut service_groups = IndexMap::new();
        let (browsers, node_data_list) = config.browsers_and_nodes();
        for browser in browsers {
            let group = ServiceGroup::new(browser);
            service_groups.insert(group.name().to_string(), group);
        }
        let mut nodes = IndexMap::new();
        for node_data in node_data_list {
            let node = RemoteNode::new(node_data)?;
            nodes.insert(node.id(), node);
        }

        Ok(Self {
            service_groups: Arc::new(RwLock::new(service_groups)),
            port_manager: Arc::new(RwLock::new(port_manager)),
            sessions: HashMap::new(),
            remote_nodes: Arc::new(RwLock::new(nodes)),
        })
    }

    pub fn port_manager(&self) -> Arc<RwLock<PortManager>> {
        self.port_manager.clone()
    }

    pub fn service_groups(&self) -> Arc<RwLock<IndexMap<ServiceGroupName, ServiceGroup>>> {
        self.service_groups.clone()
    }

    pub fn remote_nodes(&self) -> Arc<RwLock<IndexMap<NodeId, RemoteNode>>> {
        self.remote_nodes.clone()
    }

    pub fn get_session(&self, session_id: &XenonSessionId) -> Option<Arc<Mutex<Session>>> {
        self.sessions.get(session_id).cloned()
    }

    pub fn add_session(&mut self, session_id: XenonSessionId, session: Session) {
        self.sessions
            .insert(session_id, Arc::new(Mutex::new(session)));
    }

    pub fn delete_session(&mut self, session_id: &XenonSessionId) -> Option<Arc<Mutex<Session>>> {
        self.sessions.remove(session_id)
    }

    pub async fn get_timeout_sessions(&self) -> Vec<XenonSessionId> {
        let mut ids = Vec::new();
        for (xsession_id, mutex_session) in self.sessions.iter() {
            let session = mutex_session.lock().await;
            // Timeout after 30 mins.
            if session.seconds_since_last_request() > 1800 {
                ids.push(xsession_id.clone());
            }
        }
        ids
    }
}
