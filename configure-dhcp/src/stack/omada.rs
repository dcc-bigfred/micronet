//! TP-Link Omada stack: ARP OUI + hostname match + best-effort UDP discovery.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::net::{Ipv4Addr, UdpSocket};
use std::time::Duration;

use crate::stack::{Device, DeviceKind, MacAddr, Stack};
use crate::Result;

/// Well-known TP-Link OUIs used on Omada APs/controllers (first 3 octets).
const TP_LINK_OUIS: &[[u8; 3]] = &[
    [0x50, 0xc7, 0xbf],
    [0x14, 0xeb, 0xb6],
    [0x98, 0xda, 0xc4],
    [0xac, 0x84, 0xc6],
    [0xc0, 0x06, 0xc3],
    [0x60, 0x32, 0xb1],
    [0xb0, 0x95, 0x75],
    [0x00, 0x31, 0x92],
    [0x18, 0xa6, 0xf7],
    [0x54, 0xaf, 0x97],
    [0x70, 0x4f, 0x57],
    [0x90, 0x9a, 0x4a],
    [0xd8, 0x07, 0xb6],
    [0xf4, 0xf2, 0x6d],
    [0x1c, 0x61, 0xb4],
    [0x30, 0xde, 0x4b],
    [0x5c, 0xa6, 0xe6],
    [0x68, 0xff, 0x7b],
    [0x7c, 0x8b, 0xca],
    [0xb4, 0xb0, 0x24],
];

/// Omada / TP-Link discovery UDP ports (best-effort probe).
const DISCOVERY_PORTS: &[u16] = &[29810, 1040, 20002];

#[derive(Debug, Default)]
pub struct OmadaStack {
    arp_path: String,
}

impl OmadaStack {
    #[must_use]
    pub fn new() -> Self {
        Self {
            arp_path: "/proc/net/arp".to_string(),
        }
    }

    /// Test helper: read ARP from a custom path.
    #[must_use]
    pub fn with_arp_path(path: impl Into<String>) -> Self {
        Self {
            arp_path: path.into(),
        }
    }
}

impl Stack for OmadaStack {
    fn name(&self) -> &str {
        "omada"
    }

    fn detect(&self, iface: &str) -> Result<Vec<Device>> {
        let mut devices = Vec::new();
        devices.extend(scan_arp(&self.arp_path)?);
        // UDP discovery is best-effort; failures are non-fatal.
        if let Ok(extra) = udp_probe(iface) {
            for d in extra {
                if !devices.iter().any(|e| e.mac == d.mac && d.mac.is_some()) {
                    devices.push(d);
                }
            }
        }
        Ok(devices)
    }

    fn match_device(&self, hostname: &str, mac: &MacAddr) -> Option<DeviceKind> {
        if is_tp_link_oui(mac) {
            return Some(kind_from_hostname(hostname).unwrap_or(DeviceKind::AccessPoint));
        }
        kind_from_hostname(hostname)
    }
}

fn is_tp_link_oui(mac: &MacAddr) -> bool {
    let o = mac.octets();
    TP_LINK_OUIS
        .iter()
        .any(|oui| o[0] == oui[0] && o[1] == oui[1] && o[2] == oui[2])
}

fn kind_from_hostname(hostname: &str) -> Option<DeviceKind> {
    let h = hostname.to_ascii_lowercase();
    if h.contains("oc200") || h.contains("oc300") || h.contains("controller") {
        return Some(DeviceKind::Controller);
    }
    if h.contains("eap") || h.contains("omada") {
        return Some(DeviceKind::AccessPoint);
    }
    None
}

fn scan_arp(path: &str) -> Result<Vec<Device>> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(crate::Error::io_at(path, e)),
    };
    let reader = BufReader::new(file);
    let mut out = Vec::new();
    for (i, line) in reader.lines().enumerate() {
        let line = line.map_err(|e| crate::Error::io_at(path, e))?;
        if i == 0 {
            continue; // header
        }
        // IP HW type Flags HW address Mask Device
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 4 {
            continue;
        }
        let Some(mac) = MacAddr::parse(parts[3]) else {
            continue;
        };
        if !is_tp_link_oui(&mac) {
            continue;
        }
        let ip = parts[0].parse().ok();
        out.push(Device {
            stack: "omada".to_string(),
            kind: DeviceKind::AccessPoint,
            mac: Some(mac),
            ip,
            hostname: String::new(),
        });
    }
    Ok(out)
}

fn udp_probe(_iface: &str) -> Result<Vec<Device>> {
    let sock = UdpSocket::bind("0.0.0.0:0")?;
    sock.set_broadcast(true)?;
    sock.set_read_timeout(Some(Duration::from_millis(400)))?;
    // Minimal probe payload — many Omada units answer discovery noise on these ports.
    let payload: &[u8] = b"\x01\x00\x00\x00";
    for &port in DISCOVERY_PORTS {
        let addr = (Ipv4Addr::BROADCAST, port);
        let _ = sock.send_to(payload, addr);
    }
    let mut buf = [0u8; 512];
    let mut found = Vec::new();
    for _ in 0..8 {
        match sock.recv_from(&mut buf) {
            Ok((n, src)) => {
                if n == 0 {
                    continue;
                }
                found.push(Device {
                    stack: "omada".to_string(),
                    kind: DeviceKind::Unknown,
                    mac: None,
                    ip: Some(src.ip()),
                    hostname: String::new(),
                });
            }
            Err(_) => break,
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn match_oui() {
        let s = OmadaStack::new();
        let mac = MacAddr::new([0x50, 0xc7, 0xbf, 1, 2, 3]);
        assert_eq!(s.match_device("", &mac), Some(DeviceKind::AccessPoint));
        assert_eq!(
            s.match_device("OC200-Office", &MacAddr::new([0xaa, 0xbb, 0xcc, 0, 0, 1])),
            Some(DeviceKind::Controller)
        );
        assert_eq!(
            s.match_device("phone", &MacAddr::new([0xaa, 0xbb, 0xcc, 0, 0, 1])),
            None
        );
    }

    #[test]
    fn detect_from_arp_file() {
        let mut f = NamedTempFile::new().expect("tmp");
        writeln!(
            f,
            "IP address       HW type     Flags       HW address            Mask     Device\n\
10.0.10.11       0x1         0x2         50:c7:bf:11:22:33     *        eth0\n\
10.0.10.50       0x1         0x2         aa:bb:cc:11:22:33     *        eth0"
        )
        .expect("write");
        let s = OmadaStack::with_arp_path(f.path().to_string_lossy());
        let devs = s.detect("eth0").expect("detect");
        assert_eq!(devs.len(), 1);
        assert_eq!(
            devs[0].mac.as_ref().map(ToString::to_string).as_deref(),
            Some("50:c7:bf:11:22:33")
        );
    }
}
