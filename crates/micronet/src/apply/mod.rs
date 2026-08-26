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

/// DHCP server + probe, injectable in tests (must not import `ipc`).
pub trait GatewayCtl {
    fn dhcp_running(&self) -> bool;
    fn dhcp_stop(&self) -> Result<()>;
    fn dhcp_reload_or_restart(&self, cfg: &Config, iface: &str) -> Result<()>;
    fn probe_foreign_dhcp(&self, iface: &str, timeout: Duration) -> Result<bool>;
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

    fn probe_foreign_dhcp(&self, iface: &str, timeout: Duration) -> Result<bool> {
        let mac = read_mac(Path::new("/sys/class/net"), iface)?;
        probe::probe_foreign_dhcp(iface, &mac, timeout)
    }
}

/// Apply configuration to the live system (resolve, isolate, one DHCPDISCOVER, decide).
pub fn apply(cfg: &Config) -> Result<Status> {
    apply_with(cfg, &LiveNet::new(), &LiveGateway)
}

/// Apply with injected net + DHCP (unit tests).
pub fn apply_with<N: NetOps, G: GatewayCtl>(cfg: &Config, net: &N, gw: &G) -> Result<Status> {
    cfg.validate()?;
    let iface = net.resolve_iface(cfg.interface.as_deref())?;
    isolate_iface(net, &iface);
    net.bring_up(&iface)?;
    net.stop_dhclient(&iface)?;

    if gw.dhcp_running() {
        log::info!("stopping own dnsmasq before DHCP probe");
        gw.dhcp_stop()?;
    }
    let foreign_dhcp =
        gw.probe_foreign_dhcp(&iface, Duration::from_secs(cfg.probe_timeout_secs))?;

    if foreign_dhcp {
        return apply_client(cfg, net, gw, &iface);
    }

    let static_cidr = cfg.static_cidr();
    let static_ip = cfg.static_addr();
    if !net.iface_has_ipv4(&iface) {
        net.add_addr(&iface, &static_cidr)?;
    }

    let ping_target = cfg.gateway.ip;
    let gateway_reachable = if net.ipv4_addr_is_local(ping_target) {
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

/// Stop managed dnsmasq/dhclient and flush every physical Ethernet.
pub fn teardown_with<N: NetOps, G: GatewayCtl>(cfg: &Config, net: &N, gw: &G) -> Result<Status> {
    cfg.validate()?;
    // Passive: apply already admin-upped the iface; don't `ip link set up`
    // all candidates during teardown.
    let iface = net.resolve_iface_passive(cfg.interface.as_deref())?;
    gw.dhcp_stop()?;
    flush_all_ethernet(net);
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

/// Admin-down every physical Ethernet except `keep` (best-effort).
/// Already-down ifaces without an address are skipped.
fn isolate_iface<N: NetOps>(net: &N, keep: &str) {
    let names = match net.list_ethernet() {
        Ok(n) => n,
        Err(e) => {
            log::warn!("isolate: list ethernet failed ({e}); leaving other ifaces as-is");
            return;
        }
    };
    for name in names {
        if name == keep {
            continue;
        }
        if !net.link_is_up(&name) && net.iface_ipv4_cidr(&name).is_none() {
            continue;
        }
        if let Err(e) = net.stop_dhclient(&name) {
            log::debug!("isolate {name}: stop dhclient: {e}");
        }
        if let Err(e) = net.flush_addr(&name) {
            log::debug!("isolate {name}: flush: {e}");
        }
        match net.set_down(&name) {
            Ok(()) => log::info!("isolate: {name} down (keeping {keep})"),
            Err(e) => log::debug!("isolate {name}: set down: {e}"),
        }
    }
}

fn flush_all_ethernet<N: NetOps>(net: &N) {
    let names = match net.list_ethernet() {
        Ok(n) => n,
        Err(e) => {
            log::warn!("flush all ethernet: list failed ({e})");
            return;
        }
    };
    for name in names {
        if let Err(e) = net.stop_dhclient(&name) {
            log::debug!("flush {name}: stop dhclient: {e}");
        }
        if let Err(e) = net.flush_addr(&name) {
            log::debug!("flush {name}: {e}");
        }
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
    let want = cfg.gateway_cidr();
    if net.iface_ipv4_cidr(iface).as_deref() != Some(want.as_str()) {
        net.flush_addr(iface)?;
        net.add_addr(iface, &want)?;
    }
    net.del_default()?;
    gw.dhcp_reload_or_restart(cfg, iface)?;
    Ok(Status {
        mode: Mode::Gateway,
        iface: iface.to_string(),
        cidr: Some(want),
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::config::Config;
    use crate::error::Error;
    use std::collections::{HashMap, HashSet};
    use std::net::Ipv4Addr;
    use std::sync::Mutex;

    struct FakeNet {
        ping_ok: bool,
        addrs: Mutex<HashMap<String, Vec<Ipv4Addr>>>,
        dhclient: Mutex<bool>,
        default_via: Mutex<Option<Ipv4Addr>>,
        carrier: bool,
        // For auto-pick tests: ordered iface names with per-iface carrier.
        // Empty → single-iface mode (`eth0`, `carrier`).
        ifaces: Vec<String>,
        carriers: HashMap<String, bool>,
        flushed: Mutex<Vec<String>>,
        downed: Mutex<Vec<String>>,
        upped: Mutex<Vec<String>>,
        admin_up: Mutex<HashSet<String>>,
        sticky_local: Mutex<Vec<Ipv4Addr>>,
        fail_list: bool,
    }

    impl FakeNet {
        fn new(ping_ok: bool) -> Self {
            let mut admin_up = HashSet::new();
            admin_up.insert("eth0".into());
            Self {
                ping_ok,
                addrs: Mutex::new(HashMap::new()),
                dhclient: Mutex::new(false),
                default_via: Mutex::new(None),
                carrier: true,
                ifaces: Vec::new(),
                carriers: HashMap::new(),
                flushed: Mutex::new(Vec::new()),
                downed: Mutex::new(Vec::new()),
                upped: Mutex::new(Vec::new()),
                admin_up: Mutex::new(admin_up),
                sticky_local: Mutex::new(Vec::new()),
                fail_list: false,
            }
        }

        /// Multi-iface mode: `auto` picks the first iface with carrier=true.
        fn with_ifaces(ping_ok: bool, ifaces: &[(&str, bool)]) -> Self {
            let mut carriers = HashMap::new();
            let mut admin_up = HashSet::new();
            for (n, c) in ifaces {
                carriers.insert((*n).to_string(), *c);
                if *c {
                    admin_up.insert((*n).to_string());
                }
            }
            Self {
                ping_ok,
                addrs: Mutex::new(HashMap::new()),
                dhclient: Mutex::new(false),
                default_via: Mutex::new(None),
                carrier: true,
                ifaces: ifaces.iter().map(|(n, _)| (*n).to_string()).collect(),
                carriers,
                flushed: Mutex::new(Vec::new()),
                downed: Mutex::new(Vec::new()),
                upped: Mutex::new(Vec::new()),
                admin_up: Mutex::new(admin_up),
                sticky_local: Mutex::new(Vec::new()),
                fail_list: false,
            }
        }

        fn push_addr(&self, iface: &str, ip: Ipv4Addr) {
            self.addrs
                .lock()
                .unwrap()
                .entry(iface.to_string())
                .or_default()
                .push(ip);
        }
    }

    impl NetOps for FakeNet {
        fn list_ethernet(&self) -> Result<Vec<String>> {
            if self.fail_list {
                return Err(Error::NoEthernet);
            }
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
        fn bring_up(&self, iface: &str) -> Result<()> {
            if self.link_is_up(iface) {
                return Ok(());
            }
            self.upped.lock().unwrap().push(iface.to_string());
            self.admin_up.lock().unwrap().insert(iface.to_string());
            Ok(())
        }
        fn flush_addr(&self, iface: &str) -> Result<()> {
            self.flushed.lock().unwrap().push(iface.to_string());
            self.addrs.lock().unwrap().remove(iface);
            Ok(())
        }
        fn add_addr(&self, iface: &str, cidr: &str) -> Result<()> {
            let ip = cidr
                .split('/')
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(Ipv4Addr::UNSPECIFIED);
            self.push_addr(iface, ip);
            Ok(())
        }
        fn iface_has_ipv4(&self, iface: &str) -> bool {
            self.iface_ipv4_cidr(iface).is_some()
        }
        fn iface_has_addr(&self, iface: &str, ip: Ipv4Addr) -> bool {
            self.addrs
                .lock()
                .unwrap()
                .get(iface)
                .is_some_and(|v| v.contains(&ip))
        }
        fn iface_ipv4_cidr(&self, iface: &str) -> Option<String> {
            if *self.dhclient.lock().unwrap() {
                return Some("192.168.0.50/24".into());
            }
            self.addrs
                .lock()
                .unwrap()
                .get(iface)
                .and_then(|v| v.first())
                .map(|ip| format!("{ip}/{REQUIRED_PREFIX}"))
        }
        fn carrier_up(&self, iface: &str) -> bool {
            if self.ifaces.is_empty() {
                self.carrier
            } else {
                self.carriers.get(iface).copied().unwrap_or(false)
            }
        }
        fn link_is_up(&self, iface: &str) -> bool {
            self.admin_up.lock().unwrap().contains(iface)
        }
        fn set_down(&self, iface: &str) -> Result<()> {
            self.downed.lock().unwrap().push(iface.to_string());
            self.admin_up.lock().unwrap().remove(iface);
            Ok(())
        }
        fn ipv4_addr_is_local(&self, ip: Ipv4Addr) -> bool {
            if self.sticky_local.lock().unwrap().contains(&ip) {
                return true;
            }
            self.addrs.lock().unwrap().values().any(|v| v.contains(&ip))
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
        fn probe_foreign_dhcp(&self, _iface: &str, _timeout: Duration) -> Result<bool> {
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
        let s = apply_with(&cfg, &net, &gw).unwrap();
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
        let s = apply_with(&cfg, &net, &gw).unwrap();
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
        let s = apply_with(&cfg, &net, &gw).unwrap();
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
        let err = apply_with(&cfg, &net, &gw).unwrap_err();
        assert!(matches!(err, Error::DhcpProbe(_)));
        assert!(!*gw.started.lock().unwrap());
        assert!(!*gw.running.lock().unwrap());
    }

    #[test]
    fn teardown_stops_managed_state() {
        let cfg = Config::default();
        let net = FakeNet::new(false);
        let gw = FakeGw::new(false);
        *gw.running.lock().unwrap() = true;
        *net.dhclient.lock().unwrap() = true;
        net.push_addr("eth0", Ipv4Addr::new(192, 168, 0, 1));
        let s = teardown_with(&cfg, &net, &gw).unwrap();
        assert!(s.cidr.is_none());
        assert!(!*gw.running.lock().unwrap());
        assert!(!*net.dhclient.lock().unwrap());
        assert!(net.addrs.lock().unwrap().is_empty());
        assert!(net.flushed.lock().unwrap().contains(&"eth0".to_string()));
    }

    #[test]
    fn apply_auto_picks_iface_with_carrier() {
        let cfg = Config::default();
        let net = FakeNet::with_ifaces(false, &[("eth0", false), ("eth1", true)]);
        let gw = FakeGw::new(false);
        let s = apply_with(&cfg, &net, &gw).unwrap();
        assert_eq!(s.iface, "eth1");
        assert_eq!(s.mode, Mode::Gateway);
        assert_eq!(s.cidr.as_deref(), Some("192.168.0.1/24"));
        assert!(net.upped.lock().unwrap().is_empty());
    }

    #[test]
    fn teardown_auto_passive_resolves_same_iface() {
        let cfg = Config::default();
        let net = FakeNet::with_ifaces(false, &[("eth0", false), ("eth1", true)]);
        let gw = FakeGw::new(false);
        let _ = apply_with(&cfg, &net, &gw).unwrap();
        let s = teardown_with(&cfg, &net, &gw).unwrap();
        assert_eq!(s.iface, "eth1");
    }

    #[test]
    fn apply_isolates_non_selected_ethernet() {
        let cfg = Config::default();
        let net = FakeNet::with_ifaces(false, &[("eth0", false), ("eth1", true)]);
        net.push_addr("eth0", Ipv4Addr::new(192, 168, 0, 1));
        let gw = FakeGw::new(false);
        let s = apply_with(&cfg, &net, &gw).unwrap();
        assert_eq!(s.iface, "eth1");
        assert_eq!(s.mode, Mode::Gateway);
        assert_eq!(s.cidr.as_deref(), Some("192.168.0.1/24"));
        assert!(net.downed.lock().unwrap().contains(&"eth0".to_string()));
        assert!(!net.downed.lock().unwrap().contains(&"eth1".to_string()));
        assert!(net.flushed.lock().unwrap().contains(&"eth0".to_string()));
        assert!(!net.addrs.lock().unwrap().contains_key("eth0"));
        assert_eq!(
            net.addrs.lock().unwrap().get("eth1"),
            Some(&vec![Ipv4Addr::new(192, 168, 0, 1)])
        );
    }

    #[test]
    fn isolate_skips_already_down_without_addr() {
        let cfg = Config::default();
        let net = FakeNet::with_ifaces(false, &[("eth0", false), ("eth1", true)]);
        let gw = FakeGw::new(false);
        let s = apply_with(&cfg, &net, &gw).unwrap();
        assert_eq!(s.iface, "eth1");
        assert!(!net.downed.lock().unwrap().contains(&"eth0".to_string()));
    }

    #[test]
    fn apply_gateway_keeps_existing_addr() {
        let cfg = Config::default();
        let net = FakeNet::with_ifaces(false, &[("eth0", false), ("eth1", true)]);
        net.push_addr("eth1", Ipv4Addr::new(192, 168, 0, 1));
        let gw = FakeGw::new(false);
        let s = apply_with(&cfg, &net, &gw).unwrap();
        assert_eq!(s.mode, Mode::Gateway);
        assert_eq!(s.cidr.as_deref(), Some("192.168.0.1/24"));
        assert!(!net.flushed.lock().unwrap().contains(&"eth1".to_string()));
        assert_eq!(
            net.addrs.lock().unwrap().get("eth1"),
            Some(&vec![Ipv4Addr::new(192, 168, 0, 1)])
        );
        assert!(*gw.started.lock().unwrap());
    }

    #[test]
    fn gateway_ip_local_on_other_iface_is_not_reachable() {
        let cfg = Config::default();
        let net = FakeNet::new(true);
        net.sticky_local
            .lock()
            .unwrap()
            .push(Ipv4Addr::new(192, 168, 0, 1));
        let gw = FakeGw::new(false);
        let s = apply_with(&cfg, &net, &gw).unwrap();
        assert_eq!(s.mode, Mode::Gateway);
        assert_eq!(s.cidr.as_deref(), Some("192.168.0.1/24"));
        assert!(*gw.started.lock().unwrap());
        assert!(net.default_via.lock().unwrap().is_none());
    }

    #[test]
    fn isolate_survives_list_ethernet_error() {
        let cfg = Config::default();
        let mut net = FakeNet::with_ifaces(false, &[("eth0", false), ("eth1", true)]);
        net.fail_list = true;
        let gw = FakeGw::new(false);
        let s = apply_with(&cfg, &net, &gw).unwrap();
        assert_eq!(s.iface, "eth1");
        assert_eq!(s.mode, Mode::Gateway);
        assert!(net.downed.lock().unwrap().is_empty());
    }

    #[test]
    fn teardown_flushes_all_ethernet() {
        let cfg = Config::default();
        let net = FakeNet::with_ifaces(false, &[("eth0", false), ("eth1", true)]);
        net.push_addr("eth0", Ipv4Addr::new(192, 168, 0, 1));
        net.push_addr("eth1", Ipv4Addr::new(192, 168, 0, 252));
        let gw = FakeGw::new(false);
        let s = teardown_with(&cfg, &net, &gw).unwrap();
        assert!(s.cidr.is_none());
        let flushed = net.flushed.lock().unwrap().clone();
        assert!(flushed.contains(&"eth0".to_string()));
        assert!(flushed.contains(&"eth1".to_string()));
        assert!(net.addrs.lock().unwrap().is_empty());
    }
}
