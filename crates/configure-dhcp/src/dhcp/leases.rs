//! Parse dnsmasq leases and /proc/net/arp.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::net::IpAddr;
use std::path::Path;

use crate::stack::MacAddr;
use crate::{Error, Result};

#[derive(Clone, Debug)]
pub struct LeaseEntry {
    pub mac: MacAddr,
    pub ip: IpAddr,
    pub hostname: String,
}

/// dnsmasq.leases: `<expiry> <mac> <ip> <hostname> <client-id>`
pub fn parse_leases_file(path: &Path) -> Result<Vec<LeaseEntry>> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(Error::io_at(path, e)),
    };
    let mut out = Vec::new();
    for line in BufReader::new(file).lines() {
        let line = line.map_err(|e| Error::io_at(path, e))?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 4 {
            continue;
        }
        let Some(mac) = MacAddr::parse(parts[1]) else {
            continue;
        };
        let Ok(ip) = parts[2].parse::<IpAddr>() else {
            continue;
        };
        let hostname = if parts[3] == "*" {
            String::new()
        } else {
            parts[3].to_string()
        };
        out.push(LeaseEntry { mac, ip, hostname });
    }
    Ok(out)
}

#[derive(Clone, Debug)]
pub struct ArpEntry {
    pub ip: IpAddr,
    pub mac: MacAddr,
}

pub fn parse_arp_file(path: &Path) -> Result<Vec<ArpEntry>> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(Error::io_at(path, e)),
    };
    let mut out = Vec::new();
    for (i, line) in BufReader::new(file).lines().enumerate() {
        let line = line.map_err(|e| Error::io_at(path, e))?;
        if i == 0 {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 4 {
            continue;
        }
        let Ok(ip) = parts[0].parse::<IpAddr>() else {
            continue;
        };
        let Some(mac) = MacAddr::parse(parts[3]) else {
            continue;
        };
        if mac.is_zero() {
            continue;
        }
        out.push(ArpEntry { ip, mac });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn parse_lease_line() {
        let mut f = NamedTempFile::new().expect("tmp");
        writeln!(
            f,
            "1700000000 50:c7:bf:01:02:03 10.0.10.11 EAP613-Lobby 01:50:c7:bf:01:02:03"
        )
        .expect("w");
        let leases = parse_leases_file(f.path()).expect("p");
        assert_eq!(leases.len(), 1);
        assert_eq!(leases[0].hostname, "EAP613-Lobby");
        assert_eq!(leases[0].ip.to_string(), "10.0.10.11");
    }
}
