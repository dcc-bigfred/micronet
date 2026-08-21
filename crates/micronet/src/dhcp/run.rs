//! Start / SIGHUP / restart / stop dnsmasq.

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;

use crate::constants::DNSMASQ_BIN;
use crate::error::{Error, Result};

/// Fields in the main conf that SIGHUP does not re-read (dnsmasq man).
#[must_use]
pub fn restart_required(old: &str, new: &str) -> bool {
    if old == new {
        return false;
    }
    true
}

#[must_use]
pub fn is_running() -> bool {
    !dnsmasq_pids().is_empty()
}

/// Start `dnsmasq -C conf` if not already running.
pub fn start(conf: &Path) -> Result<()> {
    if !Path::new(DNSMASQ_BIN).is_file() {
        return Err(Error::DnsmasqMissing(Path::new(DNSMASQ_BIN).to_path_buf()));
    }
    if is_running() {
        return Ok(());
    }
    let status = Command::new(DNSMASQ_BIN)
        .args(["-C", &conf.to_string_lossy()])
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| Error::Other(format!("dnsmasq: {e}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::Other(format!(
            "dnsmasq exited {}",
            status.code().unwrap_or(-1)
        )))
    }
}

/// Stop all dnsmasq processes (TERM, then KILL).
pub fn stop() -> Result<()> {
    let pids = dnsmasq_pids();
    if pids.is_empty() {
        return Ok(());
    }
    for pid in &pids {
        let _ = kill(Pid::from_raw(*pid), Signal::SIGTERM);
    }
    thread::sleep(Duration::from_millis(400));
    for pid in dnsmasq_pids() {
        let _ = kill(Pid::from_raw(pid), Signal::SIGKILL);
    }
    Ok(())
}

/// Reload via SIGHUP; restart when conf changes require it or reload failed.
pub fn reload_or_restart(conf: &Path, conf_changed: bool) -> Result<()> {
    if !is_running() {
        return start(conf);
    }
    if conf_changed {
        log::info!("dnsmasq conf changed (dhcp-range/listen-address) — restart");
        stop()?;
        return start(conf);
    }
    match sighup() {
        Ok(()) => {
            thread::sleep(Duration::from_millis(150));
            if is_running() {
                Ok(())
            } else {
                log::warn!("dnsmasq vanished after SIGHUP — start");
                start(conf)
            }
        }
        Err(e) => {
            log::warn!("dnsmasq SIGHUP failed ({e}) — restart");
            stop()?;
            start(conf)
        }
    }
}

fn sighup() -> Result<()> {
    let pids = dnsmasq_pids();
    if pids.is_empty() {
        return Err(Error::Other("dnsmasq not running".into()));
    }
    for pid in pids {
        kill(Pid::from_raw(pid), Signal::SIGHUP).map_err(Error::from)?;
    }
    Ok(())
}

fn dnsmasq_pids() -> Vec<i32> {
    let Ok(entries) = fs::read_dir("/proc") else {
        return Vec::new();
    };
    let mut pids = Vec::new();
    for ent in entries.flatten() {
        let name = ent.file_name();
        let Some(pid) = name.to_str().and_then(|s| s.parse::<i32>().ok()) else {
            continue;
        };
        let cmdline = fs::read(ent.path().join("cmdline")).unwrap_or_default();
        let text = String::from_utf8_lossy(&cmdline);
        if text
            .split('\0')
            .any(|p| p == DNSMASQ_BIN || p.ends_with("/dnsmasq"))
        {
            pids.push(pid);
        }
    }
    pids
}

#[cfg(test)]
mod tests {
    use super::restart_required;

    #[test]
    fn restart_when_conf_differs() {
        assert!(restart_required("a", "b"));
        assert!(!restart_required("same", "same"));
    }
}
