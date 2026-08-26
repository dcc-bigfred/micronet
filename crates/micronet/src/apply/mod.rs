//! Mode selection and apply: client / gateway / static.

use std::net::Ipv4Addr;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::config::{
    default_dnsmasq_conf_path, default_dnsmasq_leasefile, default_dnsmasq_pidfile, Config,
};
use crate::constants::{DHCP_CLIENT_WAIT, REQUIRED_PREFIX};
use crate::dhcp;
use crate::error::Result;
use crate::net::addr::cidr_ipv4;
use crate::net::probe::{self, read_mac};
use crate::net::{LiveNet, NetOps};

/// Operating mode (illegal combinations cannot be represented).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    Client,
    Gateway,
    Static,
}

impl Mode {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Client => "client",
            Self::Gateway => "gateway",
            Self::Static => "static",
        }
    }
}

/// Snapshot returned by apply / IPC `status`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub mode: Mode,
    pub iface: String,
    /// Real IPv4 prefix (`a.b.c.d/24`) or `None` if unassigned.
    pub cidr: Option<String>,
    pub foreign_dhcp: bool,
    pub gateway_reachable: bool,
    pub dnsmasq_running: bool,
}

impl Status {
    #[must_use]
    pub fn empty() -> Self {
        Self {
            mode: Mode::Gateway,
            iface: String::new(),
            cidr: None,
            foreign_dhcp: false,
            gateway_reachable: false,
            dnsmasq_running: false,
        }
    }

    /// Cached snapshot has an IPv4 CIDR recorded (not a live check).
    #[must_use]
    pub fn is_up(&self) -> bool {
        self.cidr.is_some() && !self.iface.is_empty()
    }
}

/// Decide mode from probe results.
#[must_use]
pub fn decide(foreign_dhcp: bool, gateway_reachable: bool) -> Mode {
    if foreign_dhcp {
        Mode::Client
    } else if gateway_reachable {
        Mode::Static
    } else {
        Mode::Gateway
    }
}

/// How apply should probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbePolicy {
    /// Stop our dnsmasq, then DHCPDISCOVER + ping (start, IPC reconfigure).
    Full,
    /// Skip DHCPDISCOVER (we may be serving). Ping `gateway.ip` unless it is ours.
    SkipDhcpWhileGateway,
    /// Foreign DHCP already proved; stop our dnsmasq and start dhclient.
    BecomeClient,
}

/// DHCP server + probe, injectable in tests (must not import `ipc`).
pub trait GatewayCtl {
    fn dhcp_running(&self) -> bool;
    fn dhcp_stop(&self) -> Result<()>;
    fn dhcp_reload_or_restart(&self, cfg: &Config, iface: &str) -> Result<()>;
    fn probe_foreign_dhcp(
        &self,
        iface: &str,
        timeout: Duration,
        ignore_servers: &[Ipv4Addr],
    ) -> Result<bool>;
}

/// Live dnsmasq + DHCPDISCOVER.
pub struct LiveGateway;

impl GatewayCtl for LiveGateway {
    fn dhcp_running(&self) -> bool {
        dhcp::is_running()
    }

    fn dhcp_stop(&self) -> Result<()> {
        dhcp::stop()
    }

    fn dhcp_reload_or_restart(&self, cfg: &Config, iface: &str) -> Result<()> {
        let conf_path = default_dnsmasq_conf_path();
        let leasefile = default_dnsmasq_leasefile();
        let pidfile = default_dnsmasq_pidfile();
        if let Some(parent) = leasefile.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let body = dhcp::render_conf(cfg, iface, &leasefile, &pidfile);
        let changed = dhcp::conf::ensure_conf(&conf_path, &body)?;
        dhcp::reload_or_restart(&conf_path, changed)
    }

    fn probe_foreign_dhcp(
        &self,
        iface: &str,
        timeout: Duration,
        ignore_servers: &[Ipv4Addr],
    ) -> Result<bool> {
        let mac = read_mac(Path::new("/sys/class/net"), iface)?;
        probe::probe_foreign_dhcp(iface, &mac, timeout, ignore_servers)
    }
}

/// Apply configuration to the live system.
pub fn apply(cfg: &Config, policy: ProbePolicy) -> Result<Status> {
    apply_with(cfg, policy, &LiveNet::new(), &LiveGateway)
}

/// Apply with injected net + DHCP (unit tests).
pub fn apply_with<N: NetOps, G: GatewayCtl>(
    cfg: &Config,
    policy: ProbePolicy,
    net: &N,
    gw: &G,
) -> Result<Status> {
    cfg.validate()?;
    let iface = net.resolve_iface(cfg.interface.as_deref())?;
    net.bring_up(&iface)?;
    net.stop_dhclient(&iface)?;

    if policy == ProbePolicy::BecomeClient {
        return apply_client(cfg, net, gw, &iface);
    }

    let skip_dhcp = policy == ProbePolicy::SkipDhcpWhileGateway;
    let foreign_dhcp = if skip_dhcp {
        false
    } else {
        if gw.dhcp_running() {
            log::info!("stopping own dnsmasq before DHCP probe");
            gw.dhcp_stop()?;
        }
        gw.probe_foreign_dhcp(&iface, Duration::from_secs(cfg.probe_timeout_secs), &[])?
    };

    let static_cidr = cfg.static_cidr();
    let static_ip = cfg.static_addr();

    if foreign_dhcp {
        return apply_client(cfg, net, gw, &iface);
    }

    net.flush_addr(&iface)?;
    net.add_addr(&iface, &static_cidr)?;

    let ping_target = cfg.gateway.ip;
    let gateway_reachable = if net.iface_has_addr(&iface, ping_target) {
        false
    } else {
        net.ping(ping_target)
    };

    let mode = decide(false, gateway_reachable);
    match mode {
        Mode::Client => apply_client(cfg, net, gw, &iface),
        Mode::Static => apply_static(cfg, net, gw, &iface, static_ip),
        Mode::Gateway => apply_gateway(cfg, net, gw, &iface),
    }
}

/// Stop managed dnsmasq/dhclient and flush the resolved iface.
pub fn teardown_with<N: NetOps, G: GatewayCtl>(cfg: &Config, net: &N, gw: &G) -> Result<Status> {
    cfg.validate()?;
    // Passive: apply already admin-upped the iface; don't `ip link set up`
    // all candidates during teardown.
    let iface = net.resolve_iface_passive(cfg.interface.as_deref())?;
    gw.dhcp_stop()?;
    net.stop_dhclient(&iface)?;
    net.flush_addr(&iface)?;
    net.del_default()?;
    Ok(Status {
        mode: Mode::Gateway,
        iface,
        cidr: None,
        foreign_dhcp: false,
        gateway_reachable: false,
        dnsmasq_running: gw.dhcp_running(),
    })
}

pub fn teardown(cfg: &Config) -> Result<Status> {
    teardown_with(cfg, &LiveNet::new(), &LiveGateway)
}

/// Live liveness: empty iface fails; no carrier succeeds (avoid unplug restart loops);
/// with carrier require a live IPv4 plus the process that belongs to the mode.
#[must_use]
pub fn live_health<N: NetOps, G: GatewayCtl>(mode: Mode, iface: &str, net: &N, gw: &G) -> bool {
    if iface.is_empty() {
        return false;
    }
    if !net.carrier_up(iface) {
        return true;
    }
    if net.iface_ipv4_cidr(iface).is_none() {
        return false;
    }
    match mode {
        Mode::Gateway => gw.dhcp_running(),
        Mode::Client => net.dhclient_running(iface),
        Mode::Static => true,
    }
}

fn apply_client<N: NetOps, G: GatewayCtl>(
    cfg: &Config,
    net: &N,
    gw: &G,
    iface: &str,
) -> Result<Status> {
    let _ = cfg;
    gw.dhcp_stop()?;
    net.flush_addr(iface)?;
    net.start_dhclient(iface)?;
    let _ = net.wait_ipv4(iface, DHCP_CLIENT_WAIT);
    let cidr = net.iface_ipv4_cidr(iface);
    if cidr.is_none() {
        log::warn!("dhclient did not assign an address within {DHCP_CLIENT_WAIT:?}");
    }
    Ok(Status {
        mode: Mode::Client,
        iface: iface.to_string(),
        cidr,
        foreign_dhcp: true,
        gateway_reachable: false,
        dnsmasq_running: gw.dhcp_running(),
    })
}

fn apply_static<N: NetOps, G: GatewayCtl>(
    cfg: &Config,
    net: &N,
    gw: &G,
    iface: &str,
    static_ip: Ipv4Addr,
) -> Result<Status> {
    gw.dhcp_stop()?;
    net.replace_default_via(cfg.gateway.ip, iface)?;
    Ok(Status {
        mode: Mode::Static,
        iface: iface.to_string(),
        cidr: Some(format!("{static_ip}/{REQUIRED_PREFIX}")),
        foreign_dhcp: false,
        gateway_reachable: true,
        dnsmasq_running: gw.dhcp_running(),
    })
}

fn apply_gateway<N: NetOps, G: GatewayCtl>(
    cfg: &Config,
    net: &N,
    gw: &G,
    iface: &str,
) -> Result<Status> {
    net.flush_addr(iface)?;
    net.add_addr(iface, &cfg.gateway_cidr())?;
    net.del_default()?;
    gw.dhcp_reload_or_restart(cfg, iface)?;
    Ok(Status {
        mode: Mode::Gateway,
        iface: iface.to_string(),
        cidr: Some(cfg.gateway_cidr()),
        foreign_dhcp: false,
        gateway_reachable: false,
        dnsmasq_running: gw.dhcp_running(),
    })
}

/// Decide-only apply for unit tests (no live `ip`/`dnsmasq`).
#[must_use]
pub fn decide_status(
    iface: &str,
    foreign_dhcp: bool,
    gateway_reachable: bool,
    cfg: &Config,
) -> Status {
    let mode = decide(foreign_dhcp, gateway_reachable);
    let (cidr, dns) = match mode {
        Mode::Client => (None, false),
        Mode::Static => (Some(cfg.static_cidr()), false),
        Mode::Gateway => (Some(cfg.gateway_cidr()), true),
    };
    Status {
        mode,
        iface: iface.to_string(),
        cidr,
        foreign_dhcp,
        gateway_reachable,
        dnsmasq_running: dns,
    }
}

/// Ignore list for a periodic gateway probe: configured gateway.ip plus local inet.
#[must_use]
pub fn periodic_ignore_servers(cfg: &Config, local_cidr: Option<&str>) -> Vec<Ipv4Addr> {
    let mut ignore = vec![cfg.gateway.ip];
    if let Some(ip) = local_cidr.and_then(cidr_ipv4) {
        if !ignore.contains(&ip) {
            ignore.push(ip);
        }
    }
    ignore
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::config::Config;
    use crate::error::Error;
    use std::net::Ipv4Addr;
    use std::sync::Mutex;

    struct FakeNet {
        ping_ok: bool,
        addrs: Mutex<Vec<Ipv4Addr>>,
        dhclient: Mutex<bool>,
        default_via: Mutex<Option<Ipv4Addr>>,
        carrier: bool,
        // For auto-pick tests: ordered iface names with per-iface carrier.
        // Empty → single-iface mode (`eth0`, `carrier`).
        ifaces: Vec<String>,
        carriers: std::collections::HashMap<String, bool>,
    }

    impl FakeNet {
        fn new(ping_ok: bool) -> Self {
            Self {
                ping_ok,
                addrs: Mutex::new(Vec::new()),
                dhclient: Mutex::new(false),
                default_via: Mutex::new(None),
                carrier: true,
                ifaces: Vec::new(),
                carriers: std::collections::HashMap::new(),
            }
        }

        /// Multi-iface mode: `auto` picks the first iface with carrier=true.
        fn with_ifaces(ping_ok: bool, ifaces: &[(&str, bool)]) -> Self {
            let mut carriers = std::collections::HashMap::new();
            for (n, c) in ifaces {
                carriers.insert((*n).to_string(), *c);
            }
            Self {
                ping_ok,
                addrs: Mutex::new(Vec::new()),
                dhclient: Mutex::new(false),
                default_via: Mutex::new(None),
                carrier: true,
                ifaces: ifaces.iter().map(|(n, _)| (*n).to_string()).collect(),
                carriers,
            }
        }
    }

    impl NetOps for FakeNet {
        fn list_ethernet(&self) -> Result<Vec<String>> {
            if self.ifaces.is_empty() {
                Ok(vec!["eth0".into()])
            } else {
                Ok(self.ifaces.clone())
            }
        }
        fn is_physical_ethernet(&self, name: &str) -> bool {
            self.ifaces.is_empty() && name == "eth0" || self.ifaces.iter().any(|n| n == name)
        }
        fn resolve_iface(&self, configured: Option<&str>) -> Result<String> {
            match configured {
                None | Some("auto") => {
                    if self.ifaces.is_empty() {
                        return Ok("eth0".into());
                    }
                    // First with carrier, else first sorted.
                    let pick = self
                        .ifaces
                        .iter()
                        .find(|n| self.carriers.get(*n).copied().unwrap_or(false))
                        .cloned()
                        .or_else(|| self.ifaces.first().cloned());
                    pick.ok_or(Error::NoEthernet)
                }
                Some(name) => {
                    if !self.is_physical_ethernet(name) {
                        return Err(Error::NotEthernet(name.into()));
                    }
                    Ok(name.into())
                }
            }
        }
        fn bring_up(&self, _iface: &str) -> Result<()> {
            Ok(())
        }
        fn flush_addr(&self, _iface: &str) -> Result<()> {
            self.addrs.lock().unwrap().clear();
            Ok(())
        }
        fn add_addr(&self, _iface: &str, cidr: &str) -> Result<()> {
            let ip = cidr
                .split('/')
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(Ipv4Addr::UNSPECIFIED);
            self.addrs.lock().unwrap().push(ip);
            Ok(())
        }
        fn iface_has_ipv4(&self, iface: &str) -> bool {
            self.iface_ipv4_cidr(iface).is_some()
        }
        fn iface_has_addr(&self, _iface: &str, ip: Ipv4Addr) -> bool {
            self.addrs.lock().unwrap().contains(&ip)
        }
        fn iface_ipv4_cidr(&self, _iface: &str) -> Option<String> {
            if *self.dhclient.lock().unwrap() {
                return Some("192.168.0.50/24".into());
            }
            self.addrs
                .lock()
                .unwrap()
                .first()
                .map(|ip| format!("{ip}/{REQUIRED_PREFIX}"))
        }
        fn carrier_up(&self, _iface: &str) -> bool {
            self.carrier
        }
        fn ping(&self, _host: Ipv4Addr) -> bool {
            self.ping_ok
        }
        fn stop_dhclient(&self, _iface: &str) -> Result<()> {
            *self.dhclient.lock().unwrap() = false;
            Ok(())
        }
        fn start_dhclient(&self, _iface: &str) -> Result<()> {
            *self.dhclient.lock().unwrap() = true;
            Ok(())
        }
        fn dhclient_running(&self, _iface: &str) -> bool {
            *self.dhclient.lock().unwrap()
        }
        fn wait_ipv4(&self, iface: &str, _timeout: Duration) -> bool {
            self.iface_has_ipv4(iface)
        }
        fn replace_default_via(&self, gw: Ipv4Addr, _iface: &str) -> Result<()> {
            *self.default_via.lock().unwrap() = Some(gw);
            Ok(())
        }
        fn del_default(&self) -> Result<()> {
            *self.default_via.lock().unwrap() = None;
            Ok(())
        }
    }

    struct FakeGw {
        offer: bool,
        fail_probe: bool,
        running: Mutex<bool>,
        started: Mutex<bool>,
    }

    impl FakeGw {
        fn new(offer: bool) -> Self {
            Self {
                offer,
                fail_probe: false,
                running: Mutex::new(false),
                started: Mutex::new(false),
            }
        }
    }

    impl GatewayCtl for FakeGw {
        fn dhcp_running(&self) -> bool {
            *self.running.lock().unwrap()
        }
        fn dhcp_stop(&self) -> Result<()> {
            *self.running.lock().unwrap() = false;
            Ok(())
        }
        fn dhcp_reload_or_restart(&self, _cfg: &Config, _iface: &str) -> Result<()> {
            *self.running.lock().unwrap() = true;
            *self.started.lock().unwrap() = true;
            Ok(())
        }
        fn probe_foreign_dhcp(
            &self,
            _iface: &str,
            _timeout: Duration,
            _ignore_servers: &[Ipv4Addr],
        ) -> Result<bool> {
            if self.fail_probe {
                return Err(Error::DhcpProbe("bind failed".into()));
            }
            Ok(self.offer)
        }
    }

    #[test]
    fn three_outcomes() {
        let cfg = Config::default();
        assert_eq!(decide(true, false), Mode::Client);
        assert_eq!(decide(false, true), Mode::Static);
        assert_eq!(decide(false, false), Mode::Gateway);
        let s = decide_status("eth0", false, false, &cfg);
        assert_eq!(s.mode, Mode::Gateway);
        assert_eq!(s.cidr.as_deref(), Some("192.168.0.1/24"));
        let s = decide_status("eth0", false, true, &cfg);
        assert_eq!(s.mode, Mode::Static);
        assert_eq!(s.cidr.as_deref(), Some("192.168.0.252/24"));
        let s = decide_status("eth0", true, false, &cfg);
        assert_eq!(s.mode, Mode::Client);
    }

    #[test]
    fn apply_foreign_dhcp_is_client_with_real_cidr() {
        let cfg = Config::default();
        let net = FakeNet::new(false);
        let gw = FakeGw::new(true);
        let s = apply_with(&cfg, ProbePolicy::Full, &net, &gw).unwrap();
        assert_eq!(s.mode, Mode::Client);
        assert!(s.foreign_dhcp);
        assert!(!*gw.started.lock().unwrap());
        assert!(*net.dhclient.lock().unwrap());
        assert_eq!(s.cidr.as_deref(), Some("192.168.0.50/24"));
        assert!(!s.cidr.as_deref().unwrap().contains("dhcp"));
    }

    #[test]
    fn apply_ping_ok_is_static_252() {
        let cfg = Config::default();
        let net = FakeNet::new(true);
        let gw = FakeGw::new(false);
        let s = apply_with(&cfg, ProbePolicy::Full, &net, &gw).unwrap();
        assert_eq!(s.mode, Mode::Static);
        assert_eq!(s.cidr.as_deref(), Some("192.168.0.252/24"));
        assert_eq!(*net.default_via.lock().unwrap(), Some(cfg.gateway.ip));
        assert!(!*gw.started.lock().unwrap());
    }

    #[test]
    fn apply_no_offer_no_ping_is_gateway() {
        let cfg = Config::default();
        let net = FakeNet::new(false);
        let gw = FakeGw::new(false);
        let s = apply_with(&cfg, ProbePolicy::Full, &net, &gw).unwrap();
        assert_eq!(s.mode, Mode::Gateway);
        assert_eq!(s.cidr.as_deref(), Some("192.168.0.1/24"));
        assert!(*gw.started.lock().unwrap());
        assert!(net.default_via.lock().unwrap().is_none());
    }

    #[test]
    fn probe_err_does_not_start_dnsmasq() {
        let cfg = Config::default();
        let net = FakeNet::new(false);
        let mut gw = FakeGw::new(false);
        gw.fail_probe = true;
        let err = apply_with(&cfg, ProbePolicy::Full, &net, &gw).unwrap_err();
        assert!(matches!(err, Error::DhcpProbe(_)));
        assert!(!*gw.started.lock().unwrap());
        assert!(!*gw.running.lock().unwrap());
    }

    #[test]
    fn become_client_stops_dnsmasq_without_probe() {
        let cfg = Config::default();
        let net = FakeNet::new(false);
        let gw = FakeGw::new(false);
        *gw.running.lock().unwrap() = true;
        let s = apply_with(&cfg, ProbePolicy::BecomeClient, &net, &gw).unwrap();
        assert_eq!(s.mode, Mode::Client);
        assert!(!*gw.running.lock().unwrap());
        assert!(!*gw.started.lock().unwrap());
        assert!(*net.dhclient.lock().unwrap());
        assert_eq!(s.cidr.as_deref(), Some("192.168.0.50/24"));
    }

    #[test]
    fn teardown_stops_managed_state() {
        let cfg = Config::default();
        let net = FakeNet::new(false);
        let gw = FakeGw::new(false);
        *gw.running.lock().unwrap() = true;
        *net.dhclient.lock().unwrap() = true;
        net.addrs
            .lock()
            .unwrap()
            .push(Ipv4Addr::new(192, 168, 0, 1));
        let s = teardown_with(&cfg, &net, &gw).unwrap();
        assert!(s.cidr.is_none());
        assert!(!*gw.running.lock().unwrap());
        assert!(!*net.dhclient.lock().unwrap());
        assert!(net.addrs.lock().unwrap().is_empty());
    }

    #[test]
    fn apply_auto_picks_iface_with_carrier() {
        // eth0 has no carrier (broken/unplugged), eth1 has carrier → apply
        // must resolve to eth1 and run the gateway mode on it.
        let cfg = Config::default(); // interface: null → auto
        let net = FakeNet::with_ifaces(false, &[("eth0", false), ("eth1", true)]);
        let gw = FakeGw::new(false);
        let s = apply_with(&cfg, ProbePolicy::Full, &net, &gw).unwrap();
        assert_eq!(s.iface, "eth1");
        assert_eq!(s.mode, Mode::Gateway);
        assert_eq!(s.cidr.as_deref(), Some("192.168.0.1/24"));
    }

    #[test]
    fn teardown_auto_passive_resolves_same_iface() {
        // After apply picked eth1, teardown (passive) must resolve eth1 too,
        // without re-running `ip link set up` on all candidates.
        let cfg = Config::default();
        let net = FakeNet::with_ifaces(false, &[("eth0", false), ("eth1", true)]);
        let gw = FakeGw::new(false);
        let _ = apply_with(&cfg, ProbePolicy::Full, &net, &gw).unwrap();
        let s = teardown_with(&cfg, &net, &gw).unwrap();
        assert_eq!(s.iface, "eth1");
    }

    #[test]
    fn live_health_matrix() {
        let net = FakeNet::new(false);
        let gw = FakeGw::new(false);
        assert!(!live_health(Mode::Gateway, "", &net, &gw));

        *gw.running.lock().unwrap() = true;
        net.addrs
            .lock()
            .unwrap()
            .push(Ipv4Addr::new(192, 168, 0, 1));
        assert!(live_health(Mode::Gateway, "eth0", &net, &gw));
        *gw.running.lock().unwrap() = false;
        assert!(!live_health(Mode::Gateway, "eth0", &net, &gw));

        let mut down = FakeNet::new(false);
        down.carrier = false;
        assert!(live_health(Mode::Gateway, "eth0", &down, &gw));

        *net.dhclient.lock().unwrap() = true;
        assert!(live_health(Mode::Client, "eth0", &net, &gw));
        *net.dhclient.lock().unwrap() = false;
        net.addrs.lock().unwrap().clear();
        net.addrs
            .lock()
            .unwrap()
            .push(Ipv4Addr::new(192, 168, 0, 252));
        assert!(!live_health(Mode::Client, "eth0", &net, &gw));
        assert!(live_health(Mode::Static, "eth0", &net, &gw));
    }

    #[test]
    fn periodic_ignore_includes_gateway_and_local() {
        let cfg = Config::default();
        let v = periodic_ignore_servers(&cfg, Some("192.168.0.1/24"));
        assert_eq!(v, vec![cfg.gateway.ip]);
        let v = periodic_ignore_servers(&cfg, Some("192.168.0.50/24"));
        assert_eq!(v, vec![cfg.gateway.ip, Ipv4Addr::new(192, 168, 0, 50)]);
    }
}
