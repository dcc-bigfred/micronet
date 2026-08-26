//! Daemon loop: apply, Unix socket, inotify config reload.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::apply::{self, live_health, GatewayCtl, LiveGateway, ProbePolicy, Status};
use crate::config;
use crate::constants::{
    CARRIER_LOST_GRACE, CARRIER_REAPPLY_BACKOFF, GATEWAY_FOREIGN_DHCP_INTERVAL, STATUS_REFRESH,
};
use crate::error::{Error, Result};
use crate::ipc::{self, IpcEvent, Shared};
use crate::net::LiveNet;
use crate::net::NetOps;
use crate::signals;

/// How `micronet check` vs legacy `configure-* check` treat a missing daemon socket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckStyle {
    /// Socket down → unhealthy (microinit must restart the daemon).
    Daemon,
    /// Socket down → iface-only check (one-shot apply already exited).
    Legacy,
}

#[must_use]
pub fn check_style_from_argv0(name: &str) -> CheckStyle {
    match name {
        "configure-ethernet" | "configure-dhcp" => CheckStyle::Legacy,
        _ => CheckStyle::Daemon,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Recheck {
    Foreign(u64),
    Empty(u64),
    Failed(u64),
}

/// True when a gateway-mode recheck should yield (stop dnsmasq, become client).
#[must_use]
pub(crate) fn should_yield_gateway(current_epoch: u64, result: Recheck) -> bool {
    matches!(result, Recheck::Foreign(e) if e == current_epoch)
}

/// True when auto-picked iface should be re-resolved after sustained carrier loss.
#[must_use]
pub(crate) fn should_reapply_on_carrier_loss(
    carrier: bool,
    lost_since: Option<Instant>,
    now: Instant,
    next_allowed: Option<Instant>,
) -> bool {
    if carrier {
        return false;
    }
    let Some(since) = lost_since else {
        return false;
    };
    if now.saturating_duration_since(since) < CARRIER_LOST_GRACE {
        return false;
    }
    !matches!(next_allowed, Some(t) if now < t)
}

fn iface_is_pinned(cfg: &config::Config) -> bool {
    !matches!(cfg.interface.as_deref(), None | Some("auto"))
}

fn maybe_carrier_reapply(
    shared: &Shared,
    net: &LiveNet,
    epoch: &AtomicU64,
    lost_since: &mut Option<Instant>,
    next_allowed: &mut Option<Instant>,
) {
    let cfg = match shared.config.read() {
        Ok(c) => c.clone(),
        Err(_) => return,
    };
    if iface_is_pinned(&cfg) {
        *lost_since = None;
        return;
    }
    let iface = match shared.status.read() {
        Ok(s) => s.iface.clone(),
        Err(_) => return,
    };
    if iface.is_empty() {
        return;
    }
    let now = Instant::now();
    if net.carrier_up(&iface) {
        *lost_since = None;
        return;
    }
    if lost_since.is_none() {
        *lost_since = Some(now);
    }
    if !should_reapply_on_carrier_loss(false, *lost_since, now, *next_allowed) {
        return;
    }
    log::info!("carrier lost on {iface} for {CARRIER_LOST_GRACE:?}; re-applying");
    bump_epoch(epoch);
    match apply::apply(&cfg, ProbePolicy::Full) {
        Ok(s) => {
            log::info!("carrier re-apply → {} iface {}", s.mode.as_str(), s.iface);
            if let Ok(mut st) = shared.status.write() {
                *st = s;
            }
        }
        Err(e) => log::warn!("carrier re-apply failed: {e}"),
    }
    *next_allowed = Some(Instant::now() + CARRIER_REAPPLY_BACKOFF);
    *lost_since = None;
}

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
    let net = LiveNet::new();
    let gw = LiveGateway;
    let epoch = Arc::new(AtomicU64::new(1));
    let probe_in_flight = Arc::new(AtomicBool::new(false));
    let (recheck_tx, recheck_rx) = mpsc::channel();
    let mut next_refresh = Instant::now() + STATUS_REFRESH;
    let mut next_gateway_probe = Instant::now() + GATEWAY_FOREIGN_DHCP_INTERVAL;
    let mut carrier_lost_since: Option<Instant> = None;
    let mut next_carrier_reapply: Option<Instant> = None;
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
            bump_epoch(&epoch);
            on_config_reload(&shared, &config_path);
        }
        if event == Some(IpcEvent::Reconfigure) {
            bump_epoch(&epoch);
            on_reconfigure(&shared);
        }
        while let Ok(result) = recheck_rx.try_recv() {
            on_recheck_result(&shared, &epoch, result);
        }
        if Instant::now() >= next_refresh {
            refresh_live_status(&shared, &net, &gw);
            maybe_carrier_reapply(
                &shared,
                &net,
                &epoch,
                &mut carrier_lost_since,
                &mut next_carrier_reapply,
            );
            maybe_spawn_gateway_recheck(
                &shared,
                &net,
                &epoch,
                &probe_in_flight,
                &recheck_tx,
                &mut next_gateway_probe,
            );
            next_refresh = Instant::now() + STATUS_REFRESH;
        }
    }

    watch_stop.store(true, Ordering::SeqCst);
    let _ = std::fs::remove_file(socket);
    log::info!("micronet stopped");
    Ok(())
}

fn bump_epoch(epoch: &AtomicU64) {
    epoch.fetch_add(1, Ordering::SeqCst);
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

fn refresh_live_status(shared: &Shared, net: &LiveNet, gw: &LiveGateway) {
    let (mode, iface) = match shared.status.read() {
        Ok(s) => (s.mode, s.iface.clone()),
        Err(_) => {
            log::warn!("status lock poisoned");
            return;
        }
    };
    if iface.is_empty() {
        return;
    }
    let cidr = net.iface_ipv4_cidr(&iface);
    let dns = gw.dhcp_running();
    if let Ok(mut st) = shared.status.write() {
        if st.cidr != cidr {
            if mode == apply::Mode::Client && st.cidr.is_none() && cidr.is_some() {
                log::info!("client lease acquired on {iface}");
            }
            st.cidr = cidr;
        }
        st.dnsmasq_running = dns;
    }
}

fn maybe_spawn_gateway_recheck(
    shared: &Shared,
    net: &LiveNet,
    epoch: &Arc<AtomicU64>,
    in_flight: &Arc<AtomicBool>,
    tx: &mpsc::Sender<Recheck>,
    next_gateway_probe: &mut Instant,
) {
    if Instant::now() < *next_gateway_probe {
        return;
    }
    if in_flight.load(Ordering::SeqCst) {
        return;
    }
    let (mode, iface) = match shared.status.read() {
        Ok(s) => (s.mode, s.iface.clone()),
        Err(_) => return,
    };
    if mode != apply::Mode::Gateway || iface.is_empty() {
        return;
    }
    let cfg = match shared.config.read() {
        Ok(c) => c.clone(),
        Err(_) => return,
    };
    *next_gateway_probe = Instant::now() + GATEWAY_FOREIGN_DHCP_INTERVAL;
    let timeout = Duration::from_secs(cfg.probe_timeout_secs);
    let ignore = apply::periodic_ignore_servers(&cfg, net.iface_ipv4_cidr(&iface).as_deref());
    let epoch_n = epoch.load(Ordering::SeqCst);
    in_flight.store(true, Ordering::SeqCst);
    let tx = tx.clone();
    let in_flight_thread = Arc::clone(in_flight);
    if let Err(e) = thread::Builder::new()
        .name("dhcp-recheck".into())
        .spawn(move || {
            let result = match LiveGateway.probe_foreign_dhcp(&iface, timeout, &ignore) {
                Ok(true) => Recheck::Foreign(epoch_n),
                Ok(false) => Recheck::Empty(epoch_n),
                Err(e) => {
                    log::warn!("gateway DHCP recheck failed ({e}); staying gateway");
                    Recheck::Failed(epoch_n)
                }
            };
            in_flight_thread.store(false, Ordering::SeqCst);
            let _ = tx.send(result);
        })
    {
        log::warn!("dhcp-recheck spawn failed: {e}");
        in_flight.store(false, Ordering::SeqCst);
    }
}

fn on_recheck_result(shared: &Shared, epoch: &AtomicU64, result: Recheck) {
    let current = epoch.load(Ordering::SeqCst);
    if !should_yield_gateway(current, result) {
        return;
    }
    log::info!("foreign DHCP appeared; yielding gateway (stopping dnsmasq)");
    bump_epoch(epoch);
    let cfg = match shared.config.read() {
        Ok(c) => c.clone(),
        Err(_) => {
            log::warn!("config lock poisoned");
            return;
        }
    };
    match apply::apply(&cfg, ProbePolicy::BecomeClient) {
        Ok(s) => {
            log::info!("yielded → {}", s.mode.as_str());
            if let Ok(mut st) = shared.status.write() {
                *st = s;
            }
        }
        Err(e) => log::warn!("yield to client failed: {e}"),
    }
}

/// One-shot apply (CLI `apply` / argv0 aliases).
pub fn apply_once(config_path: &Path) -> Result<Status> {
    let cfg = config::load(config_path)?;
    apply::apply(&cfg, ProbePolicy::Full)
}

/// One-shot teardown of managed dnsmasq/dhclient/addresses.
pub fn teardown(config_path: &Path) -> Result<Status> {
    let cfg = config::load(config_path)?;
    apply::teardown(&cfg)
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

pub fn check_liveness(socket: &Path, config_path: &Path, style: CheckStyle) -> Result<bool> {
    match ipc::call(socket, &ipc::Request::Status) {
        Ok(ipc::Response::Status { mode, iface, .. }) => {
            Ok(live_health(mode, &iface, &LiveNet::new(), &LiveGateway))
        }
        Ok(_) => Ok(false),
        Err(Error::IoPath { .. }) | Err(Error::Io(_)) => {
            log::debug!("check_liveness: daemon socket unreachable");
            match style {
                CheckStyle::Daemon => Ok(false),
                CheckStyle::Legacy => legacy_iface_up(config_path),
            }
        }
        Err(e) => Err(e),
    }
}

fn legacy_iface_up(config_path: &Path) -> Result<bool> {
    let cfg = config::load(config_path)?;
    let net = LiveNet::new();
    let iface = net.resolve_iface(cfg.interface.as_deref())?;
    Ok(net.carrier_up(&iface) && net.iface_has_ipv4(&iface))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_style_from_argv0_matrix() {
        assert_eq!(check_style_from_argv0("micronet"), CheckStyle::Daemon);
        assert_eq!(
            check_style_from_argv0("configure-ethernet"),
            CheckStyle::Legacy
        );
        assert_eq!(check_style_from_argv0("configure-dhcp"), CheckStyle::Legacy);
    }

    #[test]
    fn stale_epoch_does_not_yield() {
        assert!(!should_yield_gateway(1, Recheck::Foreign(0)));
        assert!(should_yield_gateway(1, Recheck::Foreign(1)));
        assert!(!should_yield_gateway(1, Recheck::Empty(1)));
        assert!(!should_yield_gateway(1, Recheck::Failed(1)));
        assert!(!should_yield_gateway(2, Recheck::Foreign(1)));
    }

    #[test]
    fn carrier_loss_reapply_matrix() {
        let now = Instant::now() + Duration::from_secs(20);
        let lost = now - Duration::from_secs(11);
        assert!(!should_reapply_on_carrier_loss(true, Some(lost), now, None));
        assert!(!should_reapply_on_carrier_loss(false, None, now, None));
        assert!(!should_reapply_on_carrier_loss(false, Some(now), now, None));
        assert!(should_reapply_on_carrier_loss(false, Some(lost), now, None));
        assert!(!should_reapply_on_carrier_loss(
            false,
            Some(lost),
            now,
            Some(now + Duration::from_secs(5))
        ));
        let past = now - Duration::from_secs(1);
        assert!(should_reapply_on_carrier_loss(
            false,
            Some(lost),
            now,
            Some(past)
        ));
    }
}
