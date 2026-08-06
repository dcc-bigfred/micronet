//! Orchestration: gate on stacks → DHCP + reservations.

use std::path::{Path, PathBuf};

use crate::dhcp::{dnsmasq_exists, dnsmasq_running, ensure_gateway_addr, first_ethernet_iface};
use crate::dhcp::{
    ensure_base_conf, ensure_ethernet_primary, parse_arp_file, parse_leases_file, sighup_dnsmasq,
    start_dnsmasq, write_reservations, DhcpDefaults, DEFAULT_ETHERNET_CONF, DEFAULT_GATEWAY,
};
use crate::stack::{Device, MacAddr, Registry};
use crate::sticky::StickyState;
use crate::Result;

#[derive(Clone, Debug)]
pub struct Paths {
    pub state: PathBuf,
    pub ethernet_conf: PathBuf,
    pub arp: PathBuf,
}

impl Default for Paths {
    fn default() -> Self {
        Self {
            state: StickyState::path_default(),
            ethernet_conf: PathBuf::from(DEFAULT_ETHERNET_CONF),
            arp: PathBuf::from("/proc/net/arp"),
        }
    }
}

#[derive(Debug)]
pub struct UpReport {
    pub iface: String,
    pub gate_on: bool,
    pub reason: String,
    pub devices: Vec<Device>,
    pub dhcp_started: bool,
    pub reservations_changed: bool,
}

/// Run configure-dhcp up with the given registry and paths (testable).
pub fn run_up(registry: &Registry, paths: &Paths, defaults: &DhcpDefaults) -> Result<UpReport> {
    let iface = if defaults.iface.is_empty() {
        first_ethernet_iface().unwrap_or_else(|_| "eth0".to_string())
    } else {
        defaults.iface.clone()
    };

    let (detected, detect_ok) = match registry.detect_any(&iface) {
        Ok(v) => v,
        Err(e) => {
            log::warn!("detect errors: {e}");
            (vec![], false)
        }
    };

    let mut sticky = StickyState::load(&paths.state)?;
    let sticky_hit = sticky.has_any();

    let gate_on = detect_ok || sticky_hit;
    if !gate_on {
        return Ok(UpReport {
            iface,
            gate_on: false,
            reason: "no event WiFi stack detected; DHCP skipped".to_string(),
            devices: detected,
            dhcp_started: false,
            reservations_changed: false,
        });
    }

    let reason = if detect_ok {
        for d in &detected {
            sticky.remember(&d.stack);
        }
        sticky.save(&paths.state)?;
        "stack detected".to_string()
    } else {
        format!("sticky stacks: {:?}", sticky.stacks)
    };

    if !dnsmasq_exists() {
        return Err(crate::Error::DnsmasqMissing(PathBuf::from(
            crate::dhcp::run::DNSMASQ_BIN,
        )));
    }

    let _ = ensure_ethernet_primary(&paths.ethernet_conf, DEFAULT_GATEWAY)?;
    let cidr = format!("{DEFAULT_GATEWAY}/24");
    ensure_gateway_addr(&iface, &cidr)?;

    let mut d = defaults.clone();
    d.iface = iface.clone();
    let conf_changed = ensure_base_conf(&d)?;

    if !dnsmasq_running() {
        start_dnsmasq(&d.conf_path)?;
    } else if conf_changed {
        sighup_dnsmasq()?;
    }

    let mut additions: Vec<(MacAddr, String)> = Vec::new();

    // From live detect (IP known).
    for dvc in &detected {
        if let (Some(mac), Some(ip)) = (&dvc.mac, &dvc.ip) {
            additions.push((mac.clone(), ip.to_string()));
        }
    }

    // From leases + ARP matched by stacks.
    let leases = parse_leases_file(&d.leasefile).unwrap_or_default();
    for lease in leases {
        if registry.match_device(&lease.hostname, &lease.mac).is_some() {
            additions.push((lease.mac, lease.ip.to_string()));
        }
    }
    let arp = parse_arp_file(&paths.arp).unwrap_or_default();
    for ent in arp {
        if registry.match_device("", &ent.mac).is_some() {
            additions.push((ent.mac, ent.ip.to_string()));
        }
    }

    // Dedupe by MAC (last wins).
    let mut by_mac = std::collections::BTreeMap::new();
    for (mac, ip) in additions {
        by_mac.insert(mac.to_string(), (mac, ip));
    }
    let additions: Vec<_> = by_mac.into_values().collect();

    let reservations_changed = write_reservations(&d.reservations_path, &additions)?;
    if reservations_changed {
        sighup_dnsmasq()?;
    }

    Ok(UpReport {
        iface,
        gate_on: true,
        reason,
        devices: detected,
        dhcp_started: true,
        reservations_changed,
    })
}

/// check: print stacks, detection, DHCP status. Returns true if healthy when gated,
/// or true when skipped (no stack). False only on hard errors is handled by caller.
pub fn run_check(registry: &Registry, paths: &Paths, defaults: &DhcpDefaults) -> Result<i32> {
    let iface = first_ethernet_iface().unwrap_or_else(|_| defaults.iface.clone());
    println!("iface: {iface}");
    println!("registered stacks:");
    for s in registry.stacks() {
        println!("  - {}", s.name());
    }

    let (detected, detect_ok) = registry.detect_any(&iface).unwrap_or_else(|e| {
        eprintln!("detect: {e}");
        (vec![], false)
    });
    let sticky = StickyState::load(&paths.state).unwrap_or_default();
    let gate = detect_ok || sticky.has_any();

    println!("gate: {}", if gate { "ON" } else { "OFF" });
    println!("sticky: {:?}", sticky.stacks);
    println!("detected devices: {}", detected.len());
    for d in &detected {
        println!(
            "  [{}] {} mac={} ip={} host={}",
            d.stack,
            d.kind,
            d.mac
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| "-".into()),
            d.ip.map(|i| i.to_string()).unwrap_or_else(|| "-".into()),
            if d.hostname.is_empty() {
                "-"
            } else {
                &d.hostname
            }
        );
    }
    println!("dnsmasq binary: {}", dnsmasq_exists());
    println!("dnsmasq running: {}", dnsmasq_running());

    if gate && !dnsmasq_running() {
        return Ok(1);
    }
    Ok(0)
}

/// Whether path exists (for tests).
pub fn path_exists(p: &Path) -> bool {
    p.exists()
}
