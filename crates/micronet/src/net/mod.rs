//! Physical Ethernet discovery, `ip`/`ping`/`dhclient`.

use std::fs;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::config;
use crate::constants::{
    ARPHRD_ETHER, DHCLIENT_BIN, ETHTOOL_BIN, IP_BIN, PING_BIN, PING_COUNT, PING_TIMEOUT_SEC,
};
use crate::error::{Error, Result};
use crate::pidfile;

pub mod addr;
pub mod probe;

pub use addr::{host_in_slash24, IfaceAddrs};

const DEFAULT_SYS_CLASS_NET: &str = "/sys/class/net";
/// JSON `interface` sentinel: same as `null` (carrier-based pick).
const AUTO_IFACE: &str = "auto";
/// How long `resolve_auto` waits for PHY auto-negotiation after admin-up.
const AUTO_CARRIER_WAIT_MS: u64 = 2000;
/// `resolve_auto` carrier poll interval.
const AUTO_CARRIER_POLL_MS: u64 = 200;

/// Whether `resolve_auto` should `ip link set up` candidates first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinkBringUp {
    /// Production: carrier is unreadable while the iface is admin-down.
    Live,
    /// Teardown: read carrier on already-up ifaces, no `ip link set up`, no poll.
    None,
    /// Unit tests: fake sysfs already has a `carrier` file.
    #[cfg(test)]
    SysfsOnly,
}

/// Operations used by apply (real or fake in tests).
pub trait NetOps {
    fn list_ethernet(&self) -> Result<Vec<String>>;
    fn is_physical_ethernet(&self, name: &str) -> bool;
    fn resolve_iface(&self, configured: Option<&str>) -> Result<String>;
    /// Resolve without side effects (no `ip link set up` on candidates).
    /// Used by teardown, which runs after apply already admin-upped the iface.
    /// Default delegates to [`NetOps::resolve_iface`] (fakes ignore bring-up).
    fn resolve_iface_passive(&self, configured: Option<&str>) -> Result<String> {
        self.resolve_iface(configured)
    }
    fn bring_up(&self, iface: &str) -> Result<()>;
    fn flush_addr(&self, iface: &str) -> Result<()>;
    fn add_addr(&self, iface: &str, cidr: &str) -> Result<()>;
    fn iface_has_ipv4(&self, iface: &str) -> bool;
    fn iface_has_addr(&self, iface: &str, ip: Ipv4Addr) -> bool;
    fn iface_ipv4_cidr(&self, iface: &str) -> Option<String>;
    fn carrier_up(&self, iface: &str) -> bool;
    fn ping(&self, host: Ipv4Addr) -> bool;
    /// Stop the dhclient instance owned for `iface` (pidfile). Returns after daemonize, not after ACK.
    fn stop_dhclient(&self, iface: &str) -> Result<()>;
    fn start_dhclient(&self, iface: &str) -> Result<()>;
    fn dhclient_running(&self, iface: &str) -> bool;
    fn wait_ipv4(&self, iface: &str, timeout: Duration) -> bool;
    fn replace_default_via(&self, gw: Ipv4Addr, iface: &str) -> Result<()>;
    fn del_default(&self) -> Result<()>;
}

/// Live Linux netlink/`ip` implementation.
#[derive(Debug, Clone)]
pub struct LiveNet {
    sys_class_net: PathBuf,
}

impl LiveNet {
    #[must_use]
    pub fn new() -> Self {
        Self {
            sys_class_net: PathBuf::from(DEFAULT_SYS_CLASS_NET),
        }
    }

    #[must_use]
    pub fn with_sys_class_net(path: impl Into<PathBuf>) -> Self {
        Self {
            sys_class_net: path.into(),
        }
    }
}

impl Default for LiveNet {
    fn default() -> Self {
        Self::new()
    }
}

impl NetOps for LiveNet {
    fn list_ethernet(&self) -> Result<Vec<String>> {
        list_physical_ethernet(&self.sys_class_net)
    }

    fn is_physical_ethernet(&self, name: &str) -> bool {
        is_physical_ethernet(&self.sys_class_net, name)
    }

    fn resolve_iface(&self, configured: Option<&str>) -> Result<String> {
        resolve_iface(&self.sys_class_net, configured)
    }

    fn resolve_iface_passive(&self, configured: Option<&str>) -> Result<String> {
        resolve_iface_with(&self.sys_class_net, configured, LinkBringUp::None)
    }

    fn bring_up(&self, iface: &str) -> Result<()> {
        run_cmd(IP_BIN, &["link", "set", "dev", iface, "up"])?;
        apply_phy_tweaks(iface);
        Ok(())
    }

    fn flush_addr(&self, iface: &str) -> Result<()> {
        let _ = run_cmd(IP_BIN, &["addr", "flush", "dev", iface]);
        Ok(())
    }

    fn add_addr(&self, iface: &str, cidr: &str) -> Result<()> {
        match run_cmd(IP_BIN, &["addr", "add", cidr, "dev", iface]) {
            Ok(()) => Ok(()),
            Err(_) if self.iface_has_addr(iface, cidr_ip(cidr)) => Ok(()),
            Err(e) => Err(e),
        }
    }

    fn iface_has_ipv4(&self, iface: &str) -> bool {
        iface_has_ipv4(iface)
    }

    fn iface_has_addr(&self, iface: &str, ip: Ipv4Addr) -> bool {
        iface_has_addr(iface, ip)
    }

    fn iface_ipv4_cidr(&self, iface: &str) -> Option<String> {
        iface_ipv4_cidr(iface)
    }

    fn carrier_up(&self, iface: &str) -> bool {
        carrier_up(iface)
    }

    fn ping(&self, host: Ipv4Addr) -> bool {
        run_cmd(
            PING_BIN,
            &["-c", PING_COUNT, "-W", PING_TIMEOUT_SEC, &host.to_string()],
        )
        .is_ok()
    }

    fn stop_dhclient(&self, iface: &str) -> Result<()> {
        pidfile::stop(&config::dhclient_pidfile(iface), DHCLIENT_BIN)
    }

    /// Spawn `dhclient -nw`; returns after the parent daemonizes, not after DHCPACK.
    fn start_dhclient(&self, iface: &str) -> Result<()> {
        if !Path::new(DHCLIENT_BIN).is_file() {
            return Err(Error::DhclientMissing(PathBuf::from(DHCLIENT_BIN)));
        }
        let pid_path = config::dhclient_pidfile(iface);
        let lease_path = config::dhclient_leasefile(iface);
        if let Some(parent) = pid_path.parent() {
            fs::create_dir_all(parent).map_err(|e| Error::io_at(parent, e))?;
        }
        if let Some(parent) = lease_path.parent() {
            fs::create_dir_all(parent).map_err(|e| Error::io_at(parent, e))?;
        }
        let pid_s = pid_path.to_string_lossy();
        let lease_s = lease_path.to_string_lossy();
        run_cmd(
            DHCLIENT_BIN,
            &["-nw", "-pf", pid_s.as_ref(), "-lf", lease_s.as_ref(), iface],
        )
    }

    fn dhclient_running(&self, iface: &str) -> bool {
        pidfile::is_alive(&config::dhclient_pidfile(iface), DHCLIENT_BIN)
    }

    fn wait_ipv4(&self, iface: &str, timeout: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if iface_has_ipv4(iface) {
                return true;
            }
            thread::sleep(Duration::from_millis(200));
        }
        iface_has_ipv4(iface)
    }

    fn replace_default_via(&self, gw: Ipv4Addr, iface: &str) -> Result<()> {
        let gw = gw.to_string();
        run_cmd(
            IP_BIN,
            &["route", "replace", "default", "via", &gw, "dev", iface],
        )
    }

    fn del_default(&self) -> Result<()> {
        let _ = run_cmd(IP_BIN, &["route", "del", "default"]);
        Ok(())
    }
}

fn cidr_ip(cidr: &str) -> Ipv4Addr {
    addr::cidr_ipv4(cidr).unwrap_or(Ipv4Addr::UNSPECIFIED)
}

/// Pick a physical Ethernet: `null` / omitted / `"auto"` → first with carrier
/// (after a best-effort `ip link set up`); no carrier → first sorted name.
/// Any other string is an explicit device and must pass the physical filter.
pub fn resolve_iface(sys_class_net: &Path, configured: Option<&str>) -> Result<String> {
    resolve_iface_with(sys_class_net, configured, LinkBringUp::Live)
}

fn resolve_iface_with(
    sys_class_net: &Path,
    configured: Option<&str>,
    bring_up: LinkBringUp,
) -> Result<String> {
    match configured {
        None | Some(AUTO_IFACE) => resolve_auto(sys_class_net, bring_up),
        Some(name) => {
            if !is_physical_ethernet(sys_class_net, name) {
                return Err(Error::NotEthernet(name.to_string()));
            }
            Ok(name.to_string())
        }
    }
}

/// First physical Ethernet with sysfs `carrier=1`, else the first sorted name.
///
/// `bring_up=Live` admin-ups each candidate first (sysfs `carrier` is unreadable
/// while the iface is down) and polls for ~2 s to cover PHY auto-negotiation.
/// `SysfsOnly` (tests) does a single read — the fake sysfs already has `carrier`.
fn resolve_auto(sys_class_net: &Path, bring_up: LinkBringUp) -> Result<String> {
    let names = list_physical_ethernet(sys_class_net)?;
    let Some(fallback) = names.first().cloned() else {
        return Err(Error::NoEthernet);
    };
    if bring_up == LinkBringUp::Live {
        for name in &names {
            let _ = run_cmd(IP_BIN, &["link", "set", "dev", name, "up"]);
        }
        let deadline = Instant::now() + Duration::from_millis(AUTO_CARRIER_WAIT_MS);
        loop {
            if let Some(name) = names.iter().find(|n| carrier_up_sys(sys_class_net, n)) {
                log::info!("auto interface {name} (carrier)");
                return Ok(name.clone());
            }
            if Instant::now() >= deadline {
                break;
            }
            thread::sleep(Duration::from_millis(AUTO_CARRIER_POLL_MS));
        }
        log::info!(
            "auto interface {fallback} (no carrier after {AUTO_CARRIER_WAIT_MS}ms, first physical)"
        );
        return Ok(fallback);
    }
    // `None` (teardown) and `SysfsOnly` (tests): single read, no wait.
    if let Some(name) = names.iter().find(|n| carrier_up_sys(sys_class_net, n)) {
        log::info!("auto interface {name} (carrier)");
        return Ok(name.clone());
    }
    log::info!("auto interface {fallback} (no carrier, first physical)");
    Ok(fallback)
}

/// Sysfs `carrier=1` only. Admin-down typically yields an empty/error read → no carrier.
/// No `ip link` fallback (tests pass a fake sysfs; live callers admin-up first).
#[must_use]
fn carrier_up_sys(sys_class_net: &Path, name: &str) -> bool {
    let path = sys_class_net.join(name).join("carrier");
    fs::read_to_string(path).is_ok_and(|s| s.trim() == "1")
}

/// Physical Ethernet: `ARPHRD_ETHER`, not virtual, not wifi, not loopback, not bridge.
#[must_use]
pub fn is_physical_ethernet(sys_class_net: &Path, name: &str) -> bool {
    if name.is_empty() || name.contains('/') {
        return false;
    }
    let dir = sys_class_net.join(name);
    if !dir.is_dir() {
        return false;
    }
    let Ok(type_s) = fs::read_to_string(dir.join("type")) else {
        return false;
    };
    let Ok(kind) = type_s.trim().parse::<u16>() else {
        return false;
    };
    if kind != ARPHRD_ETHER {
        return false;
    }
    if dir.join("wireless").exists() {
        return false;
    }
    if dir.join("bridge").is_dir() {
        return false;
    }
    if let Ok(canon) = fs::canonicalize(&dir) {
        if canon.to_string_lossy().contains("/devices/virtual/") {
            return false;
        }
    }
    if let Ok(flags) = fs::read_to_string(dir.join("flags")) {
        if let Ok(val) = u32::from_str_radix(flags.trim().trim_start_matches("0x"), 16) {
            const IFF_LOOPBACK: u32 = 0x8;
            if val & IFF_LOOPBACK != 0 {
                return false;
            }
        }
    }
    true
}

/// Sorted physical Ethernet names under `sys_class_net`.
pub fn list_physical_ethernet(sys_class_net: &Path) -> Result<Vec<String>> {
    let entries = fs::read_dir(sys_class_net).map_err(|e| Error::io_at(sys_class_net, e))?;
    let mut names = Vec::new();
    for ent in entries {
        let ent = ent.map_err(|e| Error::io_at(sys_class_net, e))?;
        let name = ent.file_name().to_string_lossy().into_owned();
        if is_physical_ethernet(sys_class_net, &name) {
            names.push(name);
        }
    }
    names.sort();
    Ok(names)
}

pub fn iface_has_ipv4(iface: &str) -> bool {
    iface_ipv4_cidr(iface).is_some()
}

pub fn iface_has_addr(iface: &str, ip: Ipv4Addr) -> bool {
    iface_ipv4_cidr(iface)
        .as_deref()
        .is_some_and(|c| c.starts_with(&format!("{ip}/")))
}

pub fn iface_ipv4_cidr(iface: &str) -> Option<String> {
    let out = Command::new(IP_BIN)
        .args(["-4", "-o", "addr", "show", "dev", iface])
        .output()
        .ok()?;
    addr::parse_first_inet_cidr(&String::from_utf8_lossy(&out.stdout))
}

/// Carrier detected (`/sys/class/net/<iface>/carrier` or `ip link` `state UP`).
#[must_use]
pub fn carrier_up(iface: &str) -> bool {
    if carrier_up_sys(Path::new(DEFAULT_SYS_CLASS_NET), iface) {
        return true;
    }
    let Ok(out) = Command::new(IP_BIN)
        .args(["link", "show", "dev", iface])
        .output()
    else {
        return false;
    };
    String::from_utf8_lossy(&out.stdout).contains("state UP")
}

fn run_cmd(bin: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(bin)
        .args(args)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| Error::Other(format!("{bin}: {e}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::Other(format!(
            "{bin} {:?} exited {}",
            args,
            status.code().unwrap_or(-1)
        )))
    }
}

/// Best-effort PHY/offload tweaks after link up. Missing ethtool or ENOTSUP must not fail apply.
fn apply_phy_tweaks(iface: &str) {
    let eee = ethtool_eee_args(iface);
    let offload = ethtool_offload_args(iface);
    let coalesce = ethtool_coalesce_args(iface);
    for args in [eee.as_slice(), offload.as_slice(), coalesce.as_slice()] {
        if let Err(e) = run_ethtool(args) {
            log::warn!("ethtool {args:?}: {e}");
        }
    }
}

fn ethtool_eee_args(iface: &str) -> [&str; 4] {
    ["--set-eee", iface, "eee", "off"]
}

fn ethtool_offload_args(iface: &str) -> [&str; 6] {
    ["-K", iface, "tso", "off", "gso", "off"]
}

fn ethtool_coalesce_args(iface: &str) -> [&str; 6] {
    ["-C", iface, "rx-usecs", "0", "tx-usecs", "0"]
}

fn run_ethtool(args: &[&str]) -> Result<()> {
    let output = Command::new(ETHTOOL_BIN)
        .args(args)
        .output()
        .map_err(|e| Error::Other(format!("{ETHTOOL_BIN}: {e}")))?;
    if output.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&output.stderr);
        Err(Error::Other(format!(
            "{ETHTOOL_BIN} {args:?} exited {}: {}",
            output.status.code().unwrap_or(-1),
            err.trim()
        )))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use std::os::unix::fs::symlink;
    use tempfile::tempdir;

    fn write(path: &Path, body: &str) {
        if let Some(p) = path.parent() {
            fs::create_dir_all(p).unwrap();
        }
        fs::write(path, body).unwrap();
    }

    #[test]
    fn rejects_loopback_bridge_wifi_veth() {
        let dir = tempdir().unwrap();
        let sys = dir.path().join("class/net");
        fs::create_dir_all(&sys).unwrap();

        let virt_lo = dir.path().join("devices/virtual/net/lo");
        write(&virt_lo.join("type"), "772\n");
        write(&virt_lo.join("flags"), "0x9\n");
        symlink(&virt_lo, sys.join("lo")).unwrap();

        let virt_br = dir.path().join("devices/virtual/net/br0");
        write(&virt_br.join("type"), "1\n");
        fs::create_dir_all(virt_br.join("bridge")).unwrap();
        symlink(&virt_br, sys.join("br0")).unwrap();

        let virt_veth = dir.path().join("devices/virtual/net/veth0");
        write(&virt_veth.join("type"), "1\n");
        symlink(&virt_veth, sys.join("veth0")).unwrap();

        write(&sys.join("wlan0/type"), "1\n");
        fs::create_dir(sys.join("wlan0/wireless")).unwrap();

        write(&sys.join("eth0/type"), "1\n");
        write(&sys.join("eth0/flags"), "0x1003\n");

        assert!(!is_physical_ethernet(&sys, "lo"));
        assert!(!is_physical_ethernet(&sys, "br0"));
        assert!(!is_physical_ethernet(&sys, "veth0"));
        assert!(!is_physical_ethernet(&sys, "wlan0"));
        assert!(is_physical_ethernet(&sys, "eth0"));
        let list = list_physical_ethernet(&sys).unwrap();
        assert_eq!(list, vec!["eth0".to_string()]);
    }

    #[test]
    fn configured_non_ethernet_errors() {
        let dir = tempdir().unwrap();
        let sys = dir.path().join("net");
        write(&sys.join("lo/type"), "772\n");
        let err = resolve_iface_with(&sys, Some("lo"), LinkBringUp::SysfsOnly).unwrap_err();
        assert!(matches!(err, Error::NotEthernet(_)));
    }

    fn physical_eth(sys: &Path, name: &str, carrier: &str) {
        write(&sys.join(name).join("type"), "1\n");
        write(&sys.join(name).join("flags"), "0x1003\n");
        write(&sys.join(name).join("carrier"), carrier);
    }

    #[test]
    fn auto_picks_carrier_not_first_sorted() {
        let dir = tempdir().unwrap();
        let sys = dir.path().join("net");
        physical_eth(&sys, "eth0", "0\n");
        physical_eth(&sys, "eth1", "1\n");
        assert_eq!(
            resolve_iface_with(&sys, None, LinkBringUp::SysfsOnly).unwrap(),
            "eth1"
        );
        assert_eq!(
            resolve_iface_with(&sys, Some("auto"), LinkBringUp::SysfsOnly).unwrap(),
            "eth1"
        );
    }

    #[test]
    fn auto_falls_back_to_first_sorted_when_no_carrier() {
        let dir = tempdir().unwrap();
        let sys = dir.path().join("net");
        physical_eth(&sys, "eth0", "0\n");
        physical_eth(&sys, "eth1", "0\n");
        assert_eq!(
            resolve_iface_with(&sys, None, LinkBringUp::SysfsOnly).unwrap(),
            "eth0"
        );
    }

    #[test]
    fn auto_single_iface_without_carrier() {
        let dir = tempdir().unwrap();
        let sys = dir.path().join("net");
        physical_eth(&sys, "eth0", "0\n");
        assert_eq!(
            resolve_iface_with(&sys, None, LinkBringUp::SysfsOnly).unwrap(),
            "eth0"
        );
    }

    #[test]
    fn explicit_name_ignores_carrier() {
        let dir = tempdir().unwrap();
        let sys = dir.path().join("net");
        physical_eth(&sys, "eth0", "1\n");
        physical_eth(&sys, "eth1", "1\n");
        assert_eq!(
            resolve_iface_with(&sys, Some("eth1"), LinkBringUp::SysfsOnly).unwrap(),
            "eth1"
        );
    }

    #[test]
    fn auto_no_ethernet_errors() {
        let dir = tempdir().unwrap();
        let sys = dir.path().join("net");
        fs::create_dir_all(&sys).unwrap();
        let err = resolve_iface_with(&sys, None, LinkBringUp::SysfsOnly).unwrap_err();
        assert!(matches!(err, Error::NoEthernet));
    }

    #[test]
    fn phy_tweak_args_disable_eee_and_tso_gso() {
        assert_eq!(
            ethtool_eee_args("eth0"),
            ["--set-eee", "eth0", "eee", "off"]
        );
        assert_eq!(
            ethtool_offload_args("end0"),
            ["-K", "end0", "tso", "off", "gso", "off"]
        );
        assert_eq!(
            ethtool_coalesce_args("eth0"),
            ["-C", "eth0", "rx-usecs", "0", "tx-usecs", "0"]
        );
    }
}
