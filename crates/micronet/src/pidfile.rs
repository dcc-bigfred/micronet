//! Own a single process via a pidfile. Never `killall` or scan all of `/proc`.

use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;

use crate::constants::PROCESS_TERM_WAIT;
use crate::error::Result;

const DEFAULT_PROC: &str = "/proc";

/// True when `pid_path` names a live process whose cmdline still matches `bin`.
#[must_use]
pub fn is_alive(pid_path: &Path, bin: &str) -> bool {
    is_alive_in(pid_path, Path::new(DEFAULT_PROC), bin)
}

#[must_use]
pub fn is_alive_in(pid_path: &Path, proc_root: &Path, bin: &str) -> bool {
    let Some(pid) = read_pid(pid_path) else {
        return false;
    };
    let cmdline = fs::read(proc_root.join(pid.to_string()).join("cmdline")).unwrap_or_default();
    cmdline_matches(&cmdline, bin)
}

/// SIGTERM, then SIGKILL if the pidfile still points at `bin`. Missing pidfile is success.
pub fn stop(pid_path: &Path, bin: &str) -> Result<()> {
    stop_in(pid_path, Path::new(DEFAULT_PROC), bin, PROCESS_TERM_WAIT)
}

pub fn stop_in(pid_path: &Path, proc_root: &Path, bin: &str, term_wait: Duration) -> Result<()> {
    let Some(pid) = read_pid(pid_path) else {
        let _ = fs::remove_file(pid_path);
        return Ok(());
    };
    if !is_alive_in(pid_path, proc_root, bin) {
        let _ = fs::remove_file(pid_path);
        return Ok(());
    }
    let _ = kill(Pid::from_raw(pid), Signal::SIGTERM);
    thread::sleep(term_wait);
    if is_alive_in(pid_path, proc_root, bin) {
        let _ = kill(Pid::from_raw(pid), Signal::SIGKILL);
    }
    let _ = fs::remove_file(pid_path);
    Ok(())
}

#[must_use]
pub fn read_pid(pid_path: &Path) -> Option<i32> {
    let text = fs::read_to_string(pid_path).ok()?;
    let pid: i32 = text.trim().parse().ok()?;
    (pid > 0).then_some(pid)
}

#[must_use]
pub fn cmdline_matches(cmdline: &[u8], bin: &str) -> bool {
    if cmdline.is_empty() {
        return false;
    }
    let file_name = Path::new(bin)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(bin);
    let suffix = {
        let mut p = PathBuf::from("/");
        p.push(file_name);
        p
    };
    let suffix = suffix.to_string_lossy();
    cmdline.split(|&b| b == 0).any(|part| {
        if part.is_empty() {
            return false;
        }
        let text = String::from_utf8_lossy(part);
        text == bin || text == file_name || text.ends_with(suffix.as_ref())
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::tempdir;

    #[test]
    fn missing_pidfile_is_dead_and_stop_ok() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("missing.pid");
        assert!(!is_alive(&path, "/usr/sbin/dnsmasq"));
        stop(&path, "/usr/sbin/dnsmasq").unwrap();
    }

    #[test]
    fn garbage_pidfile_is_dead() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("x.pid");
        fs::write(&path, "nope\n").unwrap();
        assert!(!is_alive(&path, "dnsmasq"));
        stop(&path, "dnsmasq").unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn stale_numeric_pid_is_dead() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("x.pid");
        fs::write(&path, "999999\n").unwrap();
        assert!(!is_alive(&path, "dnsmasq"));
    }

    #[test]
    fn cmdline_matches_bin_path_and_argv0() {
        assert!(cmdline_matches(
            b"/usr/sbin/dnsmasq\0-C\0/tmp/x\0",
            "/usr/sbin/dnsmasq"
        ));
        assert!(cmdline_matches(b"dnsmasq\0", "/usr/sbin/dnsmasq"));
        assert!(!cmdline_matches(
            b"/usr/sbin/unbound\0",
            "/usr/sbin/dnsmasq"
        ));
        assert!(!cmdline_matches(b"", "/usr/sbin/dnsmasq"));
        assert!(cmdline_matches(
            b"/sbin/dhclient\0-nw\0eth0\0",
            "/sbin/dhclient"
        ));
    }

    #[test]
    fn stop_in_signals_matching_pid() {
        let dir = tempdir().unwrap();
        let proc_root = dir.path().join("proc");
        let pid_dir = proc_root.join("1234");
        fs::create_dir_all(&pid_dir).unwrap();
        fs::write(pid_dir.join("cmdline"), b"/usr/sbin/dnsmasq\0-C\0x\0").unwrap();
        // Make cmdline readable if umask is odd.
        let _ = fs::set_permissions(pid_dir.join("cmdline"), fs::Permissions::from_mode(0o644));
        let pid_path = dir.path().join("dnsmasq.pid");
        fs::write(&pid_path, "1234\n").unwrap();
        assert!(is_alive_in(&pid_path, &proc_root, "/usr/sbin/dnsmasq"));
        // SIGTERM/KILL will fail (no such process); pidfile must still be removed.
        stop_in(
            &pid_path,
            &proc_root,
            "/usr/sbin/dnsmasq",
            Duration::from_millis(0),
        )
        .unwrap();
        assert!(!pid_path.exists());
    }
}
