use crate::browser::BrowserConfig;
use serde::export::Formatter;
use serde::{Deserialize, Serialize};
use std::fmt::Display;

#[derive(Debug, Hash, Eq, PartialEq, Clone, Serialize, Deserialize)]
pub struct NodeId(String);

impl Default for NodeId {
    fn default() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }
}

impl Display for NodeId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl NodeId {
    pub fn new() -> Self {
        Self::default()
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RemoteServiceGroup {
    browser: BrowserConfig,
    remaining_sessions: u32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RemoteNodeCreate {
    service_groups: Vec<RemoteServiceGroup>,
    url: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RemoteNode {
    service_groups: Vec<RemoteServiceGroup>,
    url: String,
    id: NodeId,
}

impl RemoteNode {
    pub fn new(node_info: RemoteNodeCreate) -> Self {
        Self {
            service_groups: node_info.service_groups,
            url: node_info.url,
            id: NodeId::new(),
        }
    }

    pub fn id(&self) -> NodeId {
        self.id.clone()
    }

    pub fn update(&mut self, node: RemoteNode) {
        self.service_groups = node.service_groups;
        self.url = node.url;
    }
}
