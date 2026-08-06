//! Gate behaviour with a fake stack (no root / dnsmasq required for skip path).

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::field_reassign_with_default
)]

use configure_dhcp::dhcp::DhcpDefaults;
use configure_dhcp::run::{run_up, Paths};
use configure_dhcp::stack::{Device, DeviceKind, MacAddr, Registry, Stack};
use configure_dhcp::Result;
use tempfile::tempdir;

struct EmptyStack;

impl Stack for EmptyStack {
    fn name(&self) -> &str {
        "empty"
    }
    fn detect(&self, _: &str) -> Result<Vec<Device>> {
        Ok(vec![])
    }
    fn match_device(&self, _: &str, _: &MacAddr) -> Option<DeviceKind> {
        None
    }
}

struct HitStack;

impl Stack for HitStack {
    fn name(&self) -> &str {
        "omada"
    }
    fn detect(&self, _: &str) -> Result<Vec<Device>> {
        Ok(vec![Device {
            stack: "omada".into(),
            kind: DeviceKind::AccessPoint,
            mac: Some(MacAddr::new([0x50, 0xc7, 0xbf, 1, 2, 3])),
            ip: Some("10.0.10.11".parse().expect("ip")),
            hostname: "EAP613".into(),
        }])
    }
    fn match_device(&self, _: &str, mac: &MacAddr) -> Option<DeviceKind> {
        let o = mac.octets();
        if o[0] == 0x50 {
            Some(DeviceKind::AccessPoint)
        } else {
            None
        }
    }
}

#[test]
fn gate_off_when_no_stack() {
    let dir = tempdir().expect("tmp");
    let mut registry = Registry::new();
    registry.register(Box::new(EmptyStack));
    let paths = Paths {
        state: dir.path().join("state.json"),
        ethernet_conf: dir.path().join("eth.conf"),
        arp: dir.path().join("arp"),
    };
    let mut defaults = DhcpDefaults::default();
    defaults.iface = "lo".into();
    defaults.conf_path = dir.path().join("dnsmasq.conf");
    defaults.reservations_path = dir.path().join("res.conf");
    defaults.leasefile = dir.path().join("leases");

    let report = run_up(&registry, &paths, &defaults).expect("up");
    assert!(!report.gate_on);
    assert!(!report.dhcp_started);
}

#[test]
fn gate_on_writes_sticky_even_without_dnsmasq_binary() {
    // When Omada is detected but dnsmasq is missing, up should error after sticky/remember path
    // OR we error on missing binary. Either way gate logic ran detect.
    let dir = tempdir().expect("tmp");
    let mut registry = Registry::new();
    registry.register(Box::new(HitStack));
    let paths = Paths {
        state: dir.path().join("state.json"),
        ethernet_conf: dir.path().join("eth.conf"),
        arp: dir.path().join("arp"),
    };
    let mut defaults = DhcpDefaults::default();
    defaults.iface = "lo".into();
    defaults.conf_path = dir.path().join("dnsmasq.conf");
    defaults.reservations_path = dir.path().join("res.conf");
    defaults.leasefile = dir.path().join("leases");

    let result = run_up(&registry, &paths, &defaults);
    // On developer machines without /usr/sbin/dnsmasq this is Err(DnsmasqMissing).
    // Sticky should still be written before that check — currently sticky is written before
    // dnsmasq check. Verify sticky if Ok or if Err after sticky.
    if result.is_ok() {
        let s = configure_dhcp::sticky::StickyState::load(&paths.state).expect("load");
        assert!(s.stacks.contains("omada"));
    } else {
        // Error path: ensure detect happened (error is dnsmasq or ip).
        let err = result.expect_err("err");
        let msg = err.to_string();
        assert!(
            msg.contains("dnsmasq") || msg.contains("ip") || msg.contains("No ethernet"),
            "unexpected: {msg}"
        );
        // Sticky is saved before dnsmasq check in run_up.
        let s = configure_dhcp::sticky::StickyState::load(&paths.state).expect("load");
        assert!(s.stacks.contains("omada"));
    }
}
