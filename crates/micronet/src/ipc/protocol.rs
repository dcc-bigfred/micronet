//! IPC request/response types (camelCase JSON, `type` discriminator).

use serde::{Deserialize, Serialize};

use crate::apply::{Mode, Status};
use crate::version::Info;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    Status,
    Info,
    Reconfigure,
}

/// Daemon-owned liveness (IPC `status` + `micronet check`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Health {
    pub healthy: bool,
    pub reason: Option<String>,
}

impl Health {
    #[must_use]
    pub fn ok() -> Self {
        Self {
            healthy: true,
            reason: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
    #[serde(rename_all = "camelCase")]
    Status {
        mode: Mode,
        iface: String,
        cidr: Option<String>,
        foreign_dhcp: bool,
        gateway_reachable: bool,
        dnsmasq_running: bool,
        #[serde(default)]
        healthy: bool,
        #[serde(default)]
        unhealthy_reason: Option<String>,
    },
    Info {
        version: String,
        build_commit: String,
        #[serde(skip_serializing_if = "String::is_empty")]
        tag_commit: String,
        #[serde(skip_serializing_if = "String::is_empty")]
        build_time: String,
        hostname: String,
    },
    Ok,
    Error {
        message: String,
    },
}

impl Response {
    #[must_use]
    pub fn from_status(s: &Status, health: &Health) -> Self {
        Self::Status {
            mode: s.mode,
            iface: s.iface.clone(),
            cidr: s.cidr.clone(),
            foreign_dhcp: s.foreign_dhcp,
            gateway_reachable: s.gateway_reachable,
            dnsmasq_running: s.dnsmasq_running,
            healthy: health.healthy,
            unhealthy_reason: health.reason.clone(),
        }
    }

    #[must_use]
    pub fn from_info(info: &Info) -> Self {
        Self::Info {
            version: info.version.clone(),
            build_commit: info.build_commit.clone(),
            tag_commit: info.tag_commit.clone(),
            build_time: info.build_time.clone(),
            hostname: crate::version::hostname(),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn roundtrip_status() {
        let req = Request::Status;
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("status"));
        let back: Request = serde_json::from_str(&json).unwrap();
        assert_eq!(back, Request::Status);
    }
}
