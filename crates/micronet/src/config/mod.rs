//! `$DATA_DIR/etc/micronet.json` — camelCase, no hardcoded `/data` paths.

use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};

use ipnet::Ipv4Net;
use serde::{Deserialize, Serialize};

use crate::constants::{
    DEFAULT_LINK_RETRY_SECS, DEFAULT_PROBE_TIMEOUT_SECS, DEFAULT_RANGE_END, DEFAULT_RANGE_START,
    DEFAULT_STATIC_HOST, DEFAULT_STICKY, MAX_DNS_NAME_LEN, MAX_DNS_RECORDS, REQUIRED_PREFIX,
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

/// Optional unicast DNS records served by dnsmasq in **gateway** mode only.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DnsConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub records: Vec<DnsRecord>,
}

impl DnsConfig {
    #[must_use]
    fn is_default(&self) -> bool {
        !self.enabled && self.records.is_empty()
    }
}

/// One unicast A record. Omitted `addr` uses `gateway.ip`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DnsRecord {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub addr: Option<Ipv4Addr>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    /// Physical Ethernet name. `null` / omitted / `"auto"` → first physical
    /// Ethernet with carrier (else first sorted name); other cable Ethernet
    /// is admin-down. Any other string is an explicit device (pinned).
    #[serde(default)]
    pub interface: Option<String>,
    #[serde(default)]
    pub gateway: GatewayConfig,
    #[serde(default)]
    pub dhcp: DhcpConfig,
    #[serde(default = "default_probe_timeout")]
    pub probe_timeout_secs: u64,
    /// Seconds without carrier on the chosen iface before a full re-select.
    #[serde(default = "default_link_retry")]
    pub link_retry_secs: u64,
    /// Static unicast names. Served only while mode is `gateway`.
    #[serde(default, skip_serializing_if = "DnsConfig::is_default")]
    pub dns: DnsConfig,
}

fn default_probe_timeout() -> u64 {
    DEFAULT_PROBE_TIMEOUT_SECS
}

fn default_link_retry() -> u64 {
    DEFAULT_LINK_RETRY_SECS
}

impl Default for Config {
    fn default() -> Self {
        Self {
            interface: None,
            gateway: GatewayConfig::default(),
            dhcp: DhcpConfig::default(),
            probe_timeout_secs: default_probe_timeout(),
            link_retry_secs: default_link_retry(),
            dns: DnsConfig::default(),
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
        if self.link_retry_secs == 0 {
            return Err(Error::Config("linkRetrySecs must be > 0".into()));
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
        self.validate_dns()?;
        Ok(())
    }

    fn validate_dns(&self) -> Result<()> {
        if self.dns.records.len() > MAX_DNS_RECORDS {
            return Err(Error::Config(format!(
                "dns.records must have at most {MAX_DNS_RECORDS} entries"
            )));
        }
        let mut seen: Vec<String> = Vec::with_capacity(self.dns.records.len());
        for rec in &self.dns.records {
            let name = normalize_dns_name(&rec.name)?;
            if seen.iter().any(|n| n == &name) {
                return Err(Error::Config(format!("dns.records duplicate name {name}")));
            }
            seen.push(name);
        }
        Ok(())
    }

    /// A-record IPv4 (`addr`, or `gateway.ip` when omitted).
    #[must_use]
    pub fn dns_record_ip(&self, rec: &DnsRecord) -> Ipv4Addr {
        rec.addr.unwrap_or(self.gateway.ip)
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

/// Trim, strip a trailing root dot, lowercase. Rejects empty / illegal labels.
pub fn normalize_dns_name(name: &str) -> Result<String> {
    let trimmed = name.trim().trim_end_matches('.');
    if trimmed.is_empty() {
        return Err(Error::Config("dns record name must not be empty".into()));
    }
    if trimmed.len() > MAX_DNS_NAME_LEN {
        return Err(Error::Config(format!(
            "dns record name longer than {MAX_DNS_NAME_LEN} characters"
        )));
    }
    if trimmed.contains('\0') || trimmed.contains(' ') || trimmed.contains(',') {
        return Err(Error::Config(format!(
            "dns record name {trimmed:?} has illegal characters"
        )));
    }
    let mut out = String::with_capacity(trimmed.len());
    for (i, label) in trimmed.split('.').enumerate() {
        if label.is_empty() || label.len() > 63 {
            return Err(Error::Config(format!(
                "dns record name {trimmed:?} has an empty or overlong label"
            )));
        }
        let bytes = label.as_bytes();
        if bytes[0] == b'-' || bytes[label.len() - 1] == b'-' {
            return Err(Error::Config(format!(
                "dns record name {trimmed:?} has a label starting or ending with '-'"
            )));
        }
        if !bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'-')
        {
            return Err(Error::Config(format!(
                "dns record name {trimmed:?} is not a DNS hostname"
            )));
        }
        if i > 0 {
            out.push('.');
        }
        out.push_str(&label.to_ascii_lowercase());
    }
    Ok(out)
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
    use bigfred_shared_daemon::config::Load;
    let cfg = bigfred_shared_daemon::config::JsonFile::<Config>::new(path)
        .create_default()
        .load()
        .map_err(map_config)?;
    cfg.validate()?;
    Ok(cfg)
}

/// Load existing JSON; missing → defaults in memory (do not write).
pub fn load(path: &Path) -> Result<Config> {
    use bigfred_shared_daemon::config::Load;
    let cfg = bigfred_shared_daemon::config::JsonFile::<Config>::new(path)
        .missing_defaults()
        .load()
        .map_err(map_config)?;
    cfg.validate()?;
    Ok(cfg)
}

fn map_config(e: bigfred_shared_daemon::config::ConfigError) -> Error {
    match e {
        bigfred_shared_daemon::config::ConfigError::Io { path, source } => {
            Error::io_at(PathBuf::from(path), source)
        }
        bigfred_shared_daemon::config::ConfigError::Json(j) => Error::Json(j),
        bigfred_shared_daemon::config::ConfigError::Other(s) => Error::Other(s),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use std::fs;
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
    fn interface_auto_validates() {
        let c = Config {
            interface: Some("auto".into()),
            ..Config::default()
        };
        c.validate().unwrap();
    }

    #[test]
    fn link_retry_zero_rejected() {
        let c = Config {
            link_retry_secs: 0,
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
        assert!(!cfg.dns.enabled);
        assert!(cfg.dns.records.is_empty());
    }

    #[test]
    fn dns_json_camel_case() {
        let text = r#"{
            "dns": {
                "enabled": true,
                "records": [
                    {"name": "BigFred.lan"},
                    {"name": "wizard", "addr": "10.0.10.1"}
                ]
            },
            "gateway": {"ip": "10.0.10.1", "subnet": "10.0.10.0/24", "staticHost": 252}
        }"#;
        let cfg: Config = serde_json::from_str(text).unwrap();
        cfg.validate().unwrap();
        assert!(cfg.dns.enabled);
        assert_eq!(cfg.dns.records[0].name, "BigFred.lan");
        assert_eq!(cfg.dns_record_ip(&cfg.dns.records[0]), cfg.gateway.ip);
        assert_eq!(cfg.dns.records[1].addr, Some(Ipv4Addr::new(10, 0, 10, 1)));
        assert_eq!(normalize_dns_name("BigFred.lan").unwrap(), "bigfred.lan");
    }

    #[test]
    fn dns_bad_hostname_rejected() {
        let mut c = Config::default();
        c.dns.enabled = true;
        c.dns.records = vec![DnsRecord {
            name: "not a host".into(),
            addr: None,
        }];
        assert!(c.validate().is_err());
    }

    #[test]
    fn dns_duplicate_name_rejected() {
        let mut c = Config::default();
        c.dns.records = vec![
            DnsRecord {
                name: "wizard.lan".into(),
                addr: None,
            },
            DnsRecord {
                name: "Wizard.lan.".into(),
                addr: None,
            },
        ];
        assert!(c.validate().is_err());
    }

    #[test]
    fn dns_too_many_records_rejected() {
        let mut c = Config::default();
        c.dns.records = (0..=MAX_DNS_RECORDS)
            .map(|i| DnsRecord {
                name: format!("h{i}"),
                addr: None,
            })
            .collect();
        assert!(c.validate().is_err());
    }

    #[test]
    fn load_or_create_omits_default_dns() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("micronet.json");
        load_or_create(&path).unwrap();
        let body = fs::read_to_string(&path).unwrap();
        assert!(!body.contains("\"dns\""));
    }

    #[test]
    fn dns_addr_must_be_ipv4() {
        let text = r#"{
            "dns": {
                "enabled": true,
                "records": [{"name": "wizard.lan", "addr": "bigfred.lan"}]
            }
        }"#;
        assert!(serde_json::from_str::<Config>(text).is_err());
    }
}
