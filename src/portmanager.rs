use crate::config::XenonConfig;
use std::collections::HashMap;

pub type ServicePort = u16;

#[derive(Debug)]
pub enum PortStatus {
    Available,
    Taken,
}

#[derive(Debug)]
pub struct PortManager {
    ports: HashMap<ServicePort, PortStatus>,
}

impl PortManager {
    pub fn new(config: &XenonConfig) -> Self {
        // Parse port list.
        let port_list = config.get_port_list();
        let mut ports = HashMap::new();
        for port in port_list {
            ports.insert(port, PortStatus::Available);
        }
        Self { ports }
    }

    pub fn lock_next_port(&mut self) -> Option<ServicePort> {
        for (k, v) in self.ports.iter_mut() {
            if let PortStatus::Available = *v {
                *v = PortStatus::Taken;
                return Some(*k);
            }
        }

        None
    }

    pub fn unlock_port(&mut self, port: ServicePort) {
        if let Some(v) = self.ports.get_mut(&port) {
            *v = PortStatus::Available;
        }
    }
}
