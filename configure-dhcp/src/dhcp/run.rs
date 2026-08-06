//! Bring up gateway IP and run/reload dnsmasq.

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;

use crate::{Error, Result};

pub const DNSMASQ_BIN: &str = "/usr/sbin/dnsmasq";
pub const IP_BIN: &str = "/sbin/ip";

#[must_use]
pub fn dnsmasq_exists() -> bool {
    Path::new(DNSMASQ_BIN).is_file()
}

/// First non-wireless interface under /sys/class/net (sorted).
pub fn first_ethernet_iface() -> Result<String> {
    let dir = Path::new("/sys/class/net");
    let entries = fs::read_dir(dir).map_err(|e| Error::io_at(dir, e))?;
    let mut names = Vec::new();
    for ent in entries {
        let ent = ent.map_err(|e| Error::io_at(dir, e))?;
        let name = ent.file_name().to_string_lossy().into_owned();
        if name == "lo" {
            continue;
        }
        if dir.join(&name).join("wireless").exists() {
            continue;
        }
        names.push(name);
    }
    names.sort();
    names.into_iter().next().ok_or(Error::NoEthernet)
}

pub fn ensure_gateway_addr(iface: &str, cidr: &str) -> Result<()> {
    let _ = run_cmd(IP_BIN, &["link", "set", "dev", iface, "up"]);
    // Add address if missing (ip addr add fails if present — ignore).
    let status = Command::new(IP_BIN)
        .args(["addr", "add", cidr, "dev", iface])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(_) => Ok(()), // already present
        Err(e) => Err(Error::Other(format!("ip addr add: {e}"))),
    }
}

pub fn iface_has_ipv4(iface: &str) -> bool {
    let Ok(out) = Command::new(IP_BIN)
        .args(["-4", "addr", "show", "dev", iface])
        .output()
    else {
        return false;
    };
    String::from_utf8_lossy(&out.stdout).contains("inet ")
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

fn dnsmasq_pids() -> Vec<i32> {
    let Ok(entries) = fs::read_dir("/proc") else {
        return vec![];
    };
    let mut pids = Vec::new();
    for ent in entries.flatten() {
        let name = ent.file_name();
        let name = name.to_string_lossy();
        if !name.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let cmdline = fs::read_to_string(ent.path().join("cmdline")).unwrap_or_default();
        if cmdline.split('\0').next() == Some(DNSMASQ_BIN)
            || cmdline.contains("dnsmasq")
        {
            if let Ok(pid) = name.parse::<i32>() {
                pids.push(pid);
            }
        }
    }
    pids
}

#[must_use]
pub fn dnsmasq_running() -> bool {
    !dnsmasq_pids().is_empty()
}

pub fn start_dnsmasq(conf: &Path) -> Result<()> {
    if !dnsmasq_exists() {
        return Err(Error::DnsmasqMissing(Path::new(DNSMASQ_BIN).to_path_buf()));
    }
    if dnsmasq_running() {
        return Ok(());
    }
    run_cmd(DNSMASQ_BIN, &["-C", &conf.to_string_lossy()])
}

pub fn sighup_dnsmasq() -> Result<()> {
    let pids = dnsmasq_pids();
    if pids.is_empty() {
        return Ok(());
    }
    for pid in pids {
        kill(Pid::from_raw(pid), Signal::SIGHUP)?;
    }
    Ok(())
}
