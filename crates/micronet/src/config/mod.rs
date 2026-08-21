//! `$DATA_DIR/etc/micronet.json` — camelCase, no hardcoded `/data` paths.

use std::fs;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};

use ipnet::Ipv4Net;
use serde::{Deserialize, Serialize};

use crate::constants::{
    DEFAULT_PROBE_TIMEOUT_SECS, DEFAULT_RANGE_END, DEFAULT_RANGE_START, DEFAULT_STATIC_HOST,
    DEFAULT_STICKY, REQUIRED_PREFIX,
};
use crate::datadir;
use crate::error::{Error, Result};

pub mod watch;

/// Default config path under the data root.
#[must_use]
pub fn default_config_path() -> PathBuf {
    datadir::path(["etc", "micronet.json"])
}

/// Default control socket under the data root.
#[must_use]
pub fn default_socket_path() -> PathBuf {
    datadir::path(["run", "micronet.sock"])
}

#[must_use]
pub fn default_dnsmasq_conf_path() -> PathBuf {
    datadir::path(["etc", "dnsmasq.conf"])
}

#[must_use]
pub fn default_dnsmasq_leasefile() -> PathBuf {
    datadir::path(["etc", "dnsmasq.leases"])
}

#[must_use]
pub fn default_dnsmasq_pidfile() -> PathBuf {
    datadir::path(["run", "dnsmasq.pid"])
}

/// Per-iface dhclient pidfile (`$DATA_DIR/run/dhclient.<iface>.pid`).
#[must_use]
pub fn dhclient_pidfile(iface: &str) -> PathBuf {
    let mut p = datadir::path(["run"]);
    p.push(format!("dhclient.{iface}.pid"));
    p
}

/// Per-iface dhclient leasefile (`$DATA_DIR/etc/dhclient.<iface>.leases`).
#[must_use]
pub fn dhclient_leasefile(iface: &str) -> PathBuf {
    let mut p = datadir::path(["etc"]);
    p.push(format!("dhclient.{iface}.leases"));
    p
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GatewayConfig {
    #[serde(default = "default_gateway_ip")]
    pub ip: Ipv4Addr,
    #[serde(default = "default_subnet")]
    pub subnet: Ipv4Net,
    #[serde(default = "default_static_host")]
    pub static_host: u8,
}

fn default_gateway_ip() -> Ipv4Addr {
    Ipv4Addr::new(192, 168, 0, 1)
}

fn default_subnet() -> Ipv4Net {
    Ipv4Net::new(Ipv4Addr::new(192, 168, 0, 0), REQUIRED_PREFIX).unwrap_or_else(|_| {
        Ipv4Net::new(Ipv4Addr::UNSPECIFIED, 32).unwrap_or_else(|_| Ipv4Net::default())
    })
}

fn default_static_host() -> u8 {
    DEFAULT_STATIC_HOST
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            ip: default_gateway_ip(),
            subnet: default_subnet(),
            static_host: default_static_host(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DhcpConfig {
    #[serde(default = "default_range_start")]
    pub range_start: u8,
    #[serde(default = "default_range_end")]
    pub range_end: u8,
    /// dnsmasq lease duration (`7d`, `72h`, `3600`). Sticky MAC→IP window.
    #[serde(default = "default_sticky")]
    pub sticky: String,
}

fn default_range_start() -> u8 {
    DEFAULT_RANGE_START
}

fn default_range_end() -> u8 {
    DEFAULT_RANGE_END
}

fn default_sticky() -> String {
    DEFAULT_STICKY.to_string()
}

impl Default for DhcpConfig {
    fn default() -> Self {
        Self {
            range_start: default_range_start(),
            range_end: default_range_end(),
            sticky: default_sticky(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    /// Physical Ethernet name. `null` / omitted → first physical Ethernet.
    #[serde(default)]
    pub interface: Option<String>,
    #[serde(default)]
    pub gateway: GatewayConfig,
    #[serde(default)]
    pub dhcp: DhcpConfig,
    #[serde(default = "default_probe_timeout")]
    pub probe_timeout_secs: u64,
}

fn default_probe_timeout() -> u64 {
    DEFAULT_PROBE_TIMEOUT_SECS
}

impl Default for Config {
    fn default() -> Self {
        Self {
            interface: None,
            gateway: GatewayConfig::default(),
            dhcp: DhcpConfig::default(),
            probe_timeout_secs: default_probe_timeout(),
        }
    }
}

impl Config {
    /// Validate operator input. Returns `Err` — never `debug_assert` here.
    pub fn validate(&self) -> Result<()> {
        if self.gateway.subnet.prefix_len() != REQUIRED_PREFIX {
            return Err(Error::Config(format!(
                "gateway.subnet must be /{REQUIRED_PREFIX}"
            )));
        }
        if !self.gateway.subnet.contains(&self.gateway.ip) {
            return Err(Error::Config("gateway.ip is not in gateway.subnet".into()));
        }
        let gw_host = self.gateway.ip.octets()[3];
        if self.gateway.static_host == gw_host {
            return Err(Error::Config(
                "gateway.staticHost must differ from gateway.ip host".into(),
            ));
        }
        if self.dhcp.range_start >= self.dhcp.range_end {
            return Err(Error::Config("dhcp.rangeStart must be < rangeEnd".into()));
        }
        if self.gateway.static_host >= self.dhcp.range_start
            && self.gateway.static_host <= self.dhcp.range_end
        {
            return Err(Error::Config(
                "gateway.staticHost must be outside dhcp range".into(),
            ));
        }
        if gw_host >= self.dhcp.range_start && gw_host <= self.dhcp.range_end {
            return Err(Error::Config(
                "gateway.ip host must be outside dhcp range".into(),
            ));
        }
        parse_sticky(&self.dhcp.sticky)?;
        if self.probe_timeout_secs == 0 {
            return Err(Error::Config("probeTimeoutSecs must be > 0".into()));
        }
        if let Some(name) = &self.interface {
            if name.is_empty() {
                return Err(Error::Config("interface must not be empty".into()));
            }
            if name.contains('/') || name.contains('\0') {
                return Err(Error::Config(
                    "interface must be a simple device name".into(),
                ));
            }
        }
        Ok(())
    }

    /// Host address `.N` in `gateway.subnet` (/24).
    pub fn host_addr(&self, host: u8) -> Ipv4Addr {
        debug_assert_eq!(self.gateway.subnet.prefix_len(), REQUIRED_PREFIX);
        let o = self.gateway.subnet.network().octets();
        Ipv4Addr::new(o[0], o[1], o[2], host)
    }

    #[must_use]
    pub fn static_addr(&self) -> Ipv4Addr {
        self.host_addr(self.gateway.static_host)
    }

    #[must_use]
    pub fn range_start_addr(&self) -> Ipv4Addr {
        self.host_addr(self.dhcp.range_start)
    }

    #[must_use]
    pub fn range_end_addr(&self) -> Ipv4Addr {
        self.host_addr(self.dhcp.range_end)
    }

    #[must_use]
    pub fn gateway_cidr(&self) -> String {
        format!("{}/{}", self.gateway.ip, REQUIRED_PREFIX)
    }

    #[must_use]
    pub fn static_cidr(&self) -> String {
        format!("{}/{}", self.static_addr(), REQUIRED_PREFIX)
    }
}

/// Parse a dnsmasq duration (`7d`, `72h`, `45s`, bare seconds). Must be > 0.
pub fn parse_sticky(s: &str) -> Result<u64> {
    let s = s.trim();
    if s.is_empty() {
        return Err(Error::Config("dhcp.sticky must not be empty".into()));
    }
    let (num, mul) = if let Some(rest) = s.strip_suffix('s') {
        (rest, 1_u64)
    } else if let Some(rest) = s.strip_suffix('m') {
        (rest, 60)
    } else if let Some(rest) = s.strip_suffix('h') {
        (rest, 3600)
    } else if let Some(rest) = s.strip_suffix('d') {
        (rest, 86_400)
    } else if let Some(rest) = s.strip_suffix('w') {
        (rest, 604_800)
    } else {
        (s, 1)
    };
    let n: u64 = num
        .parse()
        .map_err(|_| Error::Config(format!("dhcp.sticky {s:?} is not a duration")))?;
    if n == 0 {
        return Err(Error::Config("dhcp.sticky must be > 0".into()));
    }
    n.checked_mul(mul)
        .ok_or_else(|| Error::Config("dhcp.sticky overflow".into()))
}

/// Load JSON; missing file → defaults written as an example (no `socket` field).
pub fn load_or_create(path: &Path) -> Result<Config> {
    match fs::read_to_string(path) {
        Ok(text) => {
            let cfg: Config = serde_json::from_str(&text)?;
            cfg.validate()?;
            Ok(cfg)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let cfg = Config::default();
            cfg.validate()?;
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|err| Error::io_at(parent, err))?;
            }
            let body = serde_json::to_string_pretty(&cfg)?;
            fs::write(path, body + "\n").map_err(|err| Error::io_at(path, err))?;
            Ok(cfg)
        }
        Err(e) => Err(Error::io_at(path, e)),
    }
}

/// Load existing JSON; missing → defaults in memory (do not write).
pub fn load(path: &Path) -> Result<Config> {
    match fs::read_to_string(path) {
        Ok(text) => {
            let cfg: Config = serde_json::from_str(&text)?;
            cfg.validate()?;
            Ok(cfg)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let cfg = Config::default();
            cfg.validate()?;
            Ok(cfg)
        }
        Err(e) => Err(Error::io_at(path, e)),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use tempfile::tempdir;

    #[test]
    fn defaults_validate() {
        Config::default().validate().unwrap();
    }

    #[test]
    fn interface_path_rejected() {
        let c = Config {
            interface: Some("eth0/evil".into()),
            ..Config::default()
        };
        assert!(c.validate().is_err());
    }

    #[test]
    fn sticky_parses() {
        assert_eq!(parse_sticky("7d").unwrap(), 7 * 86_400);
        assert_eq!(parse_sticky("72h").unwrap(), 72 * 3600);
        assert_eq!(parse_sticky("3600").unwrap(), 3600);
        assert!(parse_sticky("0").is_err());
        assert!(parse_sticky("nope").is_err());
    }

    #[test]
    fn static_host_in_pool_rejected() {
        let mut c = Config::default();
        c.gateway.static_host = 100;
        assert!(c.validate().is_err());
    }

    #[test]
    fn load_or_create_writes_without_socket() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("micronet.json");
        let cfg = load_or_create(&path).unwrap();
        assert_eq!(cfg.gateway.ip, Ipv4Addr::new(192, 168, 0, 1));
        let body = fs::read_to_string(&path).unwrap();
        assert!(!body.contains("socket"));
        assert!(body.contains("staticHost"));
    }

    #[test]
    fn json_camel_case() {
        let text = r#"{
            "gateway": {"ip": "10.0.10.1", "subnet": "10.0.10.0/24", "staticHost": 252},
            "dhcp": {"rangeStart": 50, "rangeEnd": 200, "sticky": "7d"}
        }"#;
        let cfg: Config = serde_json::from_str(text).unwrap();
        cfg.validate().unwrap();
        assert_eq!(cfg.gateway.ip, Ipv4Addr::new(10, 0, 10, 1));
        assert_eq!(cfg.static_addr(), Ipv4Addr::new(10, 0, 10, 252));
    }
}
