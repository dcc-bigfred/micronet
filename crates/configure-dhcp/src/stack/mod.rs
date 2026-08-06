//! Pluggable event WiFi stacks.

pub mod omada;

use std::fmt;
use std::net::{IpAddr, Ipv4Addr};

/// Kind of detected or matched device.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeviceKind {
    Unknown,
    Controller,
    AccessPoint,
}

impl fmt::Display for DeviceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown => write!(f, "unknown"),
            Self::Controller => write!(f, "controller"),
            Self::AccessPoint => write!(f, "ap"),
        }
    }
}

/// Hardware address (6 bytes). Empty when unknown.
#[derive(Clone, Debug, Default, Eq, PartialEq, Hash)]
pub struct MacAddr([u8; 6]);

impl MacAddr {
    #[must_use]
    pub fn new(bytes: [u8; 6]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub fn octets(&self) -> [u8; 6] {
        self.0
    }

    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.0 == [0; 6]
    }

    /// Parse `aa:bb:cc:dd:ee:ff` or `aa-bb-cc-dd-ee-ff`.
    pub fn parse(s: &str) -> Option<Self> {
        let sep = if s.contains(':') {
            ':'
        } else if s.contains('-') {
            '-'
        } else {
            return None;
        };
        let parts: Vec<&str> = s.split(sep).collect();
        if parts.len() != 6 {
            return None;
        }
        let mut bytes = [0u8; 6];
        for (i, p) in parts.iter().enumerate() {
            bytes[i] = u8::from_str_radix(p, 16).ok()?;
        }
        Some(Self(bytes))
    }
}

impl fmt::Display for MacAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            self.0[0], self.0[1], self.0[2], self.0[3], self.0[4], self.0[5]
        )
    }
}

/// Device belonging to a stack (IP/MAC may be missing before DHCP).
#[derive(Clone, Debug)]
pub struct Device {
    pub stack: String,
    pub kind: DeviceKind,
    pub mac: Option<MacAddr>,
    pub ip: Option<IpAddr>,
    pub hostname: String,
}

/// Vendor/event WiFi stack (Omada today; UniFi later).
pub trait Stack: Send + Sync {
    fn name(&self) -> &str;

    /// Probe the LAN before DHCP (L2 / discovery).
    fn detect(&self, iface: &str) -> crate::Result<Vec<Device>>;

    /// Classify a post-DHCP lease/ARP entry.
    fn match_device(&self, hostname: &str, mac: &MacAddr) -> Option<DeviceKind>;
}

/// Registry of stacks.
#[derive(Default)]
pub struct Registry {
    stacks: Vec<Box<dyn Stack>>,
}

impl Registry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, stack: Box<dyn Stack>) {
        self.stacks.push(stack);
    }

    #[must_use]
    pub fn stacks(&self) -> &[Box<dyn Stack>] {
        &self.stacks
    }

    /// Run detect on each stack; returns all devices found.
    pub fn detect_any(&self, iface: &str) -> crate::Result<(Vec<Device>, bool)> {
        let mut all = Vec::new();
        let mut last_err: Option<crate::Error> = None;
        for s in &self.stacks {
            match s.detect(iface) {
                Ok(found) => {
                    for mut d in found {
                        if d.stack.is_empty() {
                            d.stack = s.name().to_string();
                        }
                        all.push(d);
                    }
                }
                Err(e) => {
                    log::warn!("stack {}: detect: {e}", s.name());
                    last_err = Some(e);
                }
            }
        }
        let ok = !all.is_empty();
        if !ok {
            if let Some(e) = last_err {
                return Err(e);
            }
        }
        Ok((all, ok))
    }

    #[must_use]
    pub fn match_device(&self, hostname: &str, mac: &MacAddr) -> Option<(&str, DeviceKind)> {
        for s in &self.stacks {
            if let Some(kind) = s.match_device(hostname, mac) {
                return Some((s.name(), kind));
            }
        }
        None
    }
}

/// IPv4 helper used by dhcp defaults.
#[must_use]
pub fn ipv4(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(a, b, c, d))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use super::*;

    struct FakeStack {
        name: &'static str,
        devices: Vec<Device>,
        match_mac_prefix: Option<[u8; 3]>,
    }

    impl Stack for FakeStack {
        fn name(&self) -> &str {
            self.name
        }

        fn detect(&self, _iface: &str) -> crate::Result<Vec<Device>> {
            Ok(self.devices.clone())
        }

        fn match_device(&self, _hostname: &str, mac: &MacAddr) -> Option<DeviceKind> {
            let p = self.match_mac_prefix?;
            let o = mac.octets();
            if o[0] == p[0] && o[1] == p[1] && o[2] == p[2] {
                Some(DeviceKind::AccessPoint)
            } else {
                None
            }
        }
    }

    #[test]
    fn registry_detect_any() {
        let mut r = Registry::new();
        r.register(Box::new(FakeStack {
            name: "empty",
            devices: vec![],
            match_mac_prefix: None,
        }));
        r.register(Box::new(FakeStack {
            name: "omada",
            devices: vec![Device {
                stack: String::new(),
                kind: DeviceKind::AccessPoint,
                mac: Some(MacAddr::new([0x50, 0xc7, 0xbf, 1, 2, 3])),
                ip: None,
                hostname: String::new(),
            }],
            match_mac_prefix: Some([0x50, 0xc7, 0xbf]),
        }));
        let (devs, ok) = r.detect_any("eth0").expect("ok");
        assert!(ok);
        assert_eq!(devs.len(), 1);
        assert_eq!(devs[0].stack, "omada");
    }

    #[test]
    fn registry_detect_empty() {
        let mut r = Registry::new();
        r.register(Box::new(FakeStack {
            name: "empty",
            devices: vec![],
            match_mac_prefix: None,
        }));
        let (_, ok) = r.detect_any("eth0").expect("ok");
        assert!(!ok);
    }

    #[test]
    fn mac_parse_display() {
        let m = MacAddr::parse("50:C7:BF:01:02:03").expect("parse");
        assert_eq!(m.to_string(), "50:c7:bf:01:02:03");
    }
}
