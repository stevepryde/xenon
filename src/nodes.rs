use crate::browser::BrowserConfig;
use crate::error::{XenonError, XenonResult};
use crate::response::XenonResponse;
use hyper::http::uri::{Authority, Scheme};
use hyper::Uri;
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
    pub browser: BrowserConfig,
    pub remaining_sessions: u32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RemoteNodeCreate {
    #[serde(default)]
    name: String,
    url: String,
    service_groups: Vec<RemoteServiceGroup>,
}

fn parse_url(url: &str) -> Option<(Scheme, Authority)> {
    match url.parse::<Uri>() {
        Ok(uri) => {
            let scheme = uri.scheme().cloned().unwrap_or(Scheme::HTTP);
            let authority = uri.authority().cloned().unwrap_or(default_authority());
            Some((scheme, authority))
        }
        Err(_) => None,
    }
}

fn default_scheme() -> Scheme {
    Scheme::HTTP
}

fn default_authority() -> Authority {
    "localhost:8888".parse().unwrap()
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RemoteNode {
    id: NodeId,
    name: String,
    url: String,
    comms_id: u128,
    pub service_groups: Vec<RemoteServiceGroup>,
    #[serde(skip, default = "default_scheme")]
    pub scheme: Scheme,
    #[serde(skip, default = "default_authority")]
    pub authority: Authority,
}

impl RemoteNode {
    pub fn new(node_info: RemoteNodeCreate) -> XenonResult<Self> {
        let (scheme, authority) = parse_url(&node_info.url).ok_or_else(|| {
            XenonError::RespondWith(XenonResponse::ErrorRegisteringNode(format!(
                "Error parsing url for remote node: {}",
                node_info.url
            )))
        })?;

        Ok(Self {
            id: NodeId::new(),
            name: node_info.name,
            url: node_info.url,
            comms_id: 0,
            service_groups: node_info.service_groups,
            scheme,
            authority,
        })
    }

    pub fn id(&self) -> NodeId {
        self.id.clone()
    }

    pub fn display_name(&self) -> String {
        if self.name.is_empty() {
            self.id.to_string()
        } else {
            format!("{} ({})", self.name, self.id)
        }
    }

    pub fn update(&mut self, node: RemoteNode) -> XenonResult<bool> {
        // Avoid races due to network latency. Most recent update must apply.
        if node.comms_id > self.comms_id {
            let (scheme, authority) = parse_url(&node.url).ok_or_else(|| {
                XenonError::RespondWith(XenonResponse::ErrorUpdatingNode(format!(
                    "Error parsing url for remote node: {}",
                    node.url
                )))
            })?;
            self.scheme = scheme;
            self.authority = authority;
            self.name = node.name;
            self.service_groups = node.service_groups;
            self.comms_id = node.comms_id;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}
