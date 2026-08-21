//! Start / SIGHUP / restart / stop our dnsmasq (pidfile-owned).

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;

use crate::config::default_dnsmasq_pidfile;
use crate::constants::DNSMASQ_BIN;
use crate::error::{Error, Result};
use crate::pidfile;

/// Fields in the main conf that SIGHUP does not re-read (dnsmasq man).
#[must_use]
pub fn restart_required(old: &str, new: &str) -> bool {
    old != new
}

#[must_use]
pub fn is_running() -> bool {
    pidfile::is_alive(&default_dnsmasq_pidfile(), DNSMASQ_BIN)
}

/// Start `dnsmasq -C conf -x pidfile` if not already running.
pub fn start(conf: &Path) -> Result<()> {
    if !Path::new(DNSMASQ_BIN).is_file() {
        return Err(Error::DnsmasqMissing(Path::new(DNSMASQ_BIN).to_path_buf()));
    }
    if is_running() {
        return Ok(());
    }
    let pid_path = default_dnsmasq_pidfile();
    if let Some(parent) = pid_path.parent() {
        fs::create_dir_all(parent).map_err(|e| Error::io_at(parent, e))?;
    }
    if !pidfile::is_alive(&pid_path, DNSMASQ_BIN) {
        let _ = fs::remove_file(&pid_path);
    }
    let pid_s = pid_path.to_string_lossy();
    let status = Command::new(DNSMASQ_BIN)
        .args(["-C", &conf.to_string_lossy(), "-x", pid_s.as_ref()])
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

/// Stop our dnsmasq (TERM, then KILL). Missing pidfile is success.
pub fn stop() -> Result<()> {
    pidfile::stop(&default_dnsmasq_pidfile(), DNSMASQ_BIN)
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
    let pid_path = default_dnsmasq_pidfile();
    let Some(pid) = pidfile::read_pid(&pid_path) else {
        return Err(Error::Other("dnsmasq not running".into()));
    };
    if !pidfile::is_alive(&pid_path, DNSMASQ_BIN) {
        return Err(Error::Other("dnsmasq not running".into()));
    }
    kill(Pid::from_raw(pid), Signal::SIGHUP).map_err(Error::from)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::restart_required;
    use crate::constants::DNSMASQ_BIN;
    use crate::pidfile;
    use tempfile::tempdir;

    #[test]
    fn restart_when_conf_differs() {
        assert!(restart_required("a", "b"));
        assert!(!restart_required("same", "same"));
    }

    #[test]
    fn stop_on_missing_pidfile_is_ok() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("dnsmasq.pid");
        pidfile::stop(&path, DNSMASQ_BIN).unwrap();
        assert!(!pidfile::is_alive(&path, DNSMASQ_BIN));
    }
}
