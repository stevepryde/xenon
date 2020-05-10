use crate::config::XenonConfig;
use crate::session::{Session, XenonSessionId};
use std::collections::HashMap;

#[derive(Debug)]
pub struct XenonState {
    pub sessions: HashMap<XenonSessionId, Session>,
    pub config: XenonConfig,
}

impl XenonState {
    pub fn new(config: XenonConfig) -> Self {
        Self {
            sessions: HashMap::new(),
            config,
        }
    }

    pub fn get_session(&self, session_id: &XenonSessionId) -> Option<&Session> {
        self.sessions.get(session_id)
    }

    pub fn delete_session(&mut self, session_id: &XenonSessionId) -> bool {
        self.sessions.remove(session_id).is_some()
    }
}
