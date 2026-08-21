//! Mode selection and apply: client / gateway / static.

use std::net::Ipv4Addr;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::config::{default_dnsmasq_conf_path, default_dnsmasq_leasefile, Config};
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

    /// Liveness: interface has an IPv4 (CIDR recorded).
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
    /// Full DHCPDISCOVER + ping (start, IPC reconfigure, client/static reload).
    Full,
    /// Skip DHCPDISCOVER (we may be serving). Ping `gateway.ip` unless it is ours.
    SkipDhcpWhileGateway,
}

/// DHCP server + probe, injectable in tests (must not import `ipc`).
pub trait GatewayCtl {
    fn dhcp_running(&self) -> bool;
    fn dhcp_stop(&self) -> Result<()>;
    fn dhcp_reload_or_restart(&self, cfg: &Config, iface: &str) -> Result<()>;
    fn probe_foreign_dhcp(&self, iface: &str, timeout: Duration) -> bool;
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
        if let Some(parent) = leasefile.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let body = dhcp::render_conf(cfg, iface, &leasefile);
        let changed = dhcp::conf::ensure_conf(&conf_path, &body)?;
        dhcp::reload_or_restart(&conf_path, changed)
    }

    fn probe_foreign_dhcp(&self, iface: &str, timeout: Duration) -> bool {
        let mac = match read_mac(Path::new("/sys/class/net"), iface) {
            Ok(m) => m,
            Err(e) => {
                log::warn!("MAC read failed ({e}); treating as no foreign DHCP");
                return false;
            }
        };
        match probe::probe_foreign_dhcp(iface, &mac, timeout) {
            Ok(v) => v,
            Err(e) => {
                log::warn!("DHCP probe failed ({e}); treating as no offer");
                false
            }
        }
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
    net.kill_dhclient()?;

    let skip_dhcp = policy == ProbePolicy::SkipDhcpWhileGateway;
    let foreign_dhcp = if skip_dhcp {
        false
    } else {
        if gw.dhcp_running() {
            log::info!("stopping own dnsmasq before DHCP probe");
            gw.dhcp_stop()?;
        }
        gw.probe_foreign_dhcp(&iface, Duration::from_secs(cfg.probe_timeout_secs))
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
    let got = net.wait_ipv4(iface, DHCP_CLIENT_WAIT);
    let cidr = if got {
        Some(format!("{iface} dhcp"))
    } else {
        log::warn!("dhclient did not assign an address within {DHCP_CLIENT_WAIT:?}");
        None
    };
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
    }

    impl FakeNet {
        fn new(ping_ok: bool) -> Self {
            Self {
                ping_ok,
                addrs: Mutex::new(Vec::new()),
                dhclient: Mutex::new(false),
                default_via: Mutex::new(None),
            }
        }
    }

    impl NetOps for FakeNet {
        fn list_ethernet(&self) -> Result<Vec<String>> {
            Ok(vec!["eth0".into()])
        }
        fn is_physical_ethernet(&self, name: &str) -> bool {
            name == "eth0"
        }
        fn resolve_iface(&self, configured: Option<&str>) -> Result<String> {
            match configured {
                None => Ok("eth0".into()),
                Some("eth0") => Ok("eth0".into()),
                Some(n) => Err(Error::NotEthernet(n.into())),
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
        fn iface_has_ipv4(&self, _iface: &str) -> bool {
            !self.addrs.lock().unwrap().is_empty() || *self.dhclient.lock().unwrap()
        }
        fn iface_has_addr(&self, _iface: &str, ip: Ipv4Addr) -> bool {
            self.addrs.lock().unwrap().contains(&ip)
        }
        fn ping(&self, _host: Ipv4Addr) -> bool {
            self.ping_ok
        }
        fn kill_dhclient(&self) -> Result<()> {
            *self.dhclient.lock().unwrap() = false;
            Ok(())
        }
        fn start_dhclient(&self, _iface: &str) -> Result<()> {
            *self.dhclient.lock().unwrap() = true;
            Ok(())
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
        running: Mutex<bool>,
        started: Mutex<bool>,
    }

    impl FakeGw {
        fn new(offer: bool) -> Self {
            Self {
                offer,
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
        fn probe_foreign_dhcp(&self, _iface: &str, _timeout: Duration) -> bool {
            self.offer
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
    fn apply_foreign_dhcp_is_client() {
        let cfg = Config::default();
        let net = FakeNet::new(false);
        let gw = FakeGw::new(true);
        let s = apply_with(&cfg, ProbePolicy::Full, &net, &gw).unwrap();
        assert_eq!(s.mode, Mode::Client);
        assert!(s.foreign_dhcp);
        assert!(!*gw.started.lock().unwrap());
        assert!(*net.dhclient.lock().unwrap());
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
}
