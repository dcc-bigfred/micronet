//! Daemon loop: apply, Unix socket, inotify config reload.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::apply::{self, ProbePolicy, Status};
use crate::config;
use crate::error::{Error, Result};
use crate::ipc::{self, IpcEvent, Shared};
use crate::signals;

/// Run until SIGTERM/SIGINT.
pub fn run(config_path: &Path, socket: &Path) -> Result<()> {
    let cfg = config::load_or_create(config_path)?;
    log::info!("config {}", config_path.display());

    let status = match apply::apply(&cfg, ProbePolicy::Full) {
        Ok(s) => {
            log::info!(
                "mode {} iface {} cidr {:?}",
                s.mode.as_str(),
                s.iface,
                s.cidr
            );
            s
        }
        Err(e) => {
            log::error!("initial apply failed: {e}");
            Status::empty()
        }
    };

    let shared = Shared::new(cfg, status);
    let (ev_tx, ev_rx) = mpsc::channel();
    ipc::serve(socket, Arc::clone(&shared), ev_tx)?;

    let (reload_rx, watch_stop) = config::watch::spawn(config_path.to_path_buf())?;
    let stop = Arc::new(AtomicBool::new(false));
    signals::install(&stop)?;

    let config_path = config_path.to_path_buf();
    loop {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        let mut event = None;
        match ev_rx.recv_timeout(Duration::from_millis(200)) {
            Ok(e) => event = Some(e),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if reload_rx.try_recv().is_ok() {
            on_config_reload(&shared, &config_path);
        }
        if event == Some(IpcEvent::Reconfigure) {
            on_reconfigure(&shared);
        }
        thread::sleep(Duration::from_millis(0));
    }

    watch_stop.store(true, Ordering::SeqCst);
    let _ = std::fs::remove_file(socket);
    log::info!("micronet stopped");
    Ok(())
}

fn on_config_reload(shared: &Shared, path: &Path) {
    match config::load(path) {
        Ok(new_cfg) => {
            let prev_mode = shared
                .status
                .read()
                .map(|s| s.mode)
                .unwrap_or(apply::Mode::Gateway);
            {
                match shared.config.write() {
                    Ok(mut g) => *g = new_cfg.clone(),
                    Err(_) => {
                        log::warn!("config lock poisoned; keeping previous");
                        return;
                    }
                }
            }
            let policy = if prev_mode == apply::Mode::Gateway {
                ProbePolicy::SkipDhcpWhileGateway
            } else {
                ProbePolicy::Full
            };
            match apply::apply(&new_cfg, policy) {
                Ok(s) => {
                    if let Ok(mut st) = shared.status.write() {
                        *st = s;
                    }
                    log::info!("config reloaded");
                }
                Err(e) => log::warn!("apply after reload failed: {e}"),
            }
        }
        Err(e) => {
            log::warn!("invalid config {}, keeping previous: {e}", path.display());
        }
    }
}

fn on_reconfigure(shared: &Shared) {
    let cfg = match shared.config.read() {
        Ok(c) => c.clone(),
        Err(_) => {
            log::warn!("config lock poisoned");
            return;
        }
    };
    match apply::apply(&cfg, ProbePolicy::Full) {
        Ok(s) => {
            log::info!("reconfigure → {}", s.mode.as_str());
            if let Ok(mut st) = shared.status.write() {
                *st = s;
            }
        }
        Err(e) => log::warn!("reconfigure failed: {e}"),
    }
}

/// One-shot apply (CLI `apply` / argv0 aliases).
pub fn apply_once(config_path: &Path) -> Result<Status> {
    let cfg = config::load(config_path)?;
    apply::apply(&cfg, ProbePolicy::Full)
}

/// Resolve `--socket`: relative joined under data root; absolute kept.
#[must_use]
pub fn resolve_socket(cli: Option<&PathBuf>) -> PathBuf {
    match cli {
        None => ipc::default_socket(),
        Some(p) if p.is_absolute() => p.clone(),
        Some(p) => crate::datadir::root().join(p),
    }
}

/// Resolve `--config`: relative joined under data root.
#[must_use]
pub fn resolve_config(cli: Option<&PathBuf>) -> PathBuf {
    match cli {
        None => config::default_config_path(),
        Some(p) if p.is_absolute() => p.clone(),
        Some(p) => crate::datadir::root().join(p),
    }
}

pub fn check_liveness(socket: &Path) -> Result<bool> {
    match ipc::call(socket, &ipc::Request::Status) {
        Ok(ipc::Response::Status { cidr, iface, .. }) => Ok(cidr.is_some() && !iface.is_empty()),
        Ok(_) => Ok(false),
        Err(Error::IoPath { .. }) | Err(Error::Io(_)) => Ok(false),
        Err(e) => Err(e),
    }
}
