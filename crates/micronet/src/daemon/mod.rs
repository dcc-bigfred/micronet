//! Daemon loop: apply once, watch the chosen link, Unix socket, inotify reload.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::apply::{self, GatewayCtl, LiveGateway, Mode, Status};
use crate::config;
use crate::constants::{HEALTH_FAIL_THRESHOLD, STATUS_REFRESH};
use crate::error::{Error, Result};
use crate::ipc::{self, Health, IpcEvent, Shared};
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

struct LinkState {
    down_since: Option<Instant>,
}

/// Liveness verdict computed from live facts (no I/O).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Verdict {
    Healthy,
    Unhealthy(&'static str),
}

/// Process that belongs to the current mode is missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HealAction {
    None,
    RestartDnsmasq,
    RestartDhclient,
}

/// True when the chosen iface has been without carrier long enough to re-select.
#[must_use]
pub(crate) fn should_reselect(
    carrier: bool,
    down_since: Option<Instant>,
    now: Instant,
    retry: Duration,
) -> bool {
    !carrier && down_since.is_some_and(|t| now.saturating_duration_since(t) >= retry)
}

#[must_use]
pub(crate) fn assess(
    mode: Mode,
    carrier: bool,
    has_ipv4: bool,
    mode_proc_running: bool,
) -> Verdict {
    if !carrier {
        return Verdict::Healthy;
    }
    if !has_ipv4 {
        return Verdict::Unhealthy("carrier but no IPv4");
    }
    match mode {
        Mode::Gateway if !mode_proc_running => Verdict::Unhealthy("gateway without dnsmasq"),
        Mode::Client if !mode_proc_running => Verdict::Unhealthy("client without dhclient"),
        _ => Verdict::Healthy,
    }
}

#[must_use]
pub(crate) fn should_report_unhealthy(streak: u32) -> bool {
    streak >= HEALTH_FAIL_THRESHOLD
}

#[must_use]
pub(crate) fn heal_action(mode: Mode, proc_running: bool) -> HealAction {
    match mode {
        Mode::Gateway if !proc_running => HealAction::RestartDnsmasq,
        Mode::Client if !proc_running => HealAction::RestartDhclient,
        _ => HealAction::None,
    }
}

/// Run until SIGTERM/SIGINT.
pub fn run(config_path: &Path, socket: &Path) -> Result<()> {
    let cfg = config::load_or_create(config_path)?;
    log::info!("config {}", config_path.display());

    let status = match apply::apply(&cfg) {
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

    let mut link = LinkState { down_since: None };
    let shared = Shared::new(cfg, status);
    let (ev_tx, ev_rx) = mpsc::channel();
    ipc::serve(socket, Arc::clone(&shared), ev_tx)?;

    let (reload_rx, watch_stop) = config::watch::spawn(config_path.to_path_buf())?;
    let stop = Arc::new(AtomicBool::new(false));
    signals::install(&stop)?;

    let config_path = config_path.to_path_buf();
    let net = LiveNet::new();
    let gw = LiveGateway;
    let mut next_refresh = Instant::now() + STATUS_REFRESH;
    let mut fail_streak: u32 = 0;
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
            on_config_reload(&shared, &config_path, &mut link, &mut fail_streak);
        }
        if event == Some(IpcEvent::Reconfigure) {
            on_reconfigure(&shared, &mut link, &mut fail_streak);
        }
        if Instant::now() >= next_refresh {
            tick(&shared, &net, &gw, &mut link, &mut fail_streak);
            next_refresh = Instant::now() + STATUS_REFRESH;
        }
    }

    watch_stop.store(true, Ordering::SeqCst);
    let _ = std::fs::remove_file(socket);
    log::info!("micronet stopped");
    Ok(())
}

fn apply_now(shared: &Shared, cfg: &config::Config) -> Result<Status> {
    shared.applying.store(true, Ordering::SeqCst);
    let out = apply::apply(cfg);
    shared.applying.store(false, Ordering::SeqCst);
    out
}

fn store_status(shared: &Shared, s: Status, link: &mut LinkState) {
    link.down_since = None;
    if let Ok(mut st) = shared.status.write() {
        *st = s;
    }
}

fn on_config_reload(shared: &Shared, path: &Path, link: &mut LinkState, fail_streak: &mut u32) {
    match config::load(path) {
        Ok(new_cfg) => {
            {
                match shared.config.write() {
                    Ok(mut g) => *g = new_cfg.clone(),
                    Err(_) => {
                        log::warn!("config lock poisoned; keeping previous");
                        return;
                    }
                }
            }
            match apply_now(shared, &new_cfg) {
                Ok(s) => {
                    store_status(shared, s, link);
                    *fail_streak = 0;
                    set_health(shared, true, None);
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

fn on_reconfigure(shared: &Shared, link: &mut LinkState, fail_streak: &mut u32) {
    let cfg = match shared.config.read() {
        Ok(c) => c.clone(),
        Err(_) => {
            log::warn!("config lock poisoned");
            return;
        }
    };
    match apply_now(shared, &cfg) {
        Ok(s) => {
            log::info!("reconfigure → {}", s.mode.as_str());
            store_status(shared, s, link);
            *fail_streak = 0;
            set_health(shared, true, None);
        }
        Err(e) => log::warn!("reconfigure failed: {e}"),
    }
}

fn tick(
    shared: &Shared,
    net: &LiveNet,
    gw: &LiveGateway,
    link: &mut LinkState,
    fail_streak: &mut u32,
) {
    if shared.applying.load(Ordering::SeqCst) {
        *fail_streak = 0;
        set_health(shared, true, None);
        return;
    }

    let (mode, iface) = match shared.status.read() {
        Ok(s) => (s.mode, s.iface.clone()),
        Err(_) => {
            log::warn!("status lock poisoned");
            return;
        }
    };
    if iface.is_empty() {
        record_verdict(shared, Verdict::Unhealthy("empty iface"), fail_streak);
        return;
    }

    let cfg = match shared.config.read() {
        Ok(c) => c.clone(),
        Err(_) => return,
    };
    let retry = Duration::from_secs(cfg.link_retry_secs);
    let carrier = net.carrier_up(&iface);
    let now = Instant::now();

    if carrier {
        if link.down_since.is_some() {
            log::info!("link restored on {iface}");
        }
        link.down_since = None;
        refresh_live_status(shared, net, gw, &iface);
        maybe_self_heal(mode, &iface, &cfg, net, gw);
        refresh_live_status(shared, net, gw, &iface);
        let has_ipv4 = net.iface_ipv4_cidr(&iface).is_some();
        let proc_ok = match mode {
            Mode::Gateway => gw.dhcp_running(),
            Mode::Client => net.dhclient_running(&iface),
            Mode::Static => true,
        };
        record_verdict(shared, assess(mode, true, has_ipv4, proc_ok), fail_streak);
        return;
    }

    if link.down_since.is_none() {
        log::info!(
            "link down on {iface}; retrying for {}s",
            cfg.link_retry_secs
        );
        link.down_since = Some(now);
    }
    *fail_streak = 0;
    set_health(shared, true, None);

    if !should_reselect(false, link.down_since, now, retry) {
        return;
    }
    log::info!("link not restored; re-selecting interface");
    match apply_now(shared, &cfg) {
        Ok(s) => {
            log::info!("re-select → {} iface {}", s.mode.as_str(), s.iface);
            store_status(shared, s, link);
            *fail_streak = 0;
            set_health(shared, true, None);
        }
        Err(e) => log::warn!("re-select failed: {e}"),
    }
}

fn refresh_live_status(shared: &Shared, net: &LiveNet, gw: &LiveGateway, iface: &str) {
    let cidr = net.iface_ipv4_cidr(iface);
    let dns = gw.dhcp_running();
    if let Ok(mut st) = shared.status.write() {
        if st.cidr != cidr {
            if st.mode == Mode::Client && st.cidr.is_none() && cidr.is_some() {
                log::info!("client lease acquired on {iface}");
            }
            st.cidr = cidr;
        }
        st.dnsmasq_running = dns;
    }
}

fn maybe_self_heal(mode: Mode, iface: &str, cfg: &config::Config, net: &LiveNet, gw: &LiveGateway) {
    let proc_ok = match mode {
        Mode::Gateway => gw.dhcp_running(),
        Mode::Client => net.dhclient_running(iface),
        Mode::Static => true,
    };
    match heal_action(mode, proc_ok) {
        HealAction::None => {}
        HealAction::RestartDnsmasq => {
            log::warn!("dnsmasq missing; restarting");
            if let Err(e) = gw.dhcp_reload_or_restart(cfg, iface) {
                log::warn!("dnsmasq restart failed: {e}");
            }
        }
        HealAction::RestartDhclient => {
            log::warn!("dhclient missing; restarting");
            if let Err(e) = net.start_dhclient(iface) {
                log::warn!("dhclient restart failed: {e}");
            }
        }
    }
}

fn record_verdict(shared: &Shared, verdict: Verdict, streak: &mut u32) {
    match verdict {
        Verdict::Healthy => {
            *streak = 0;
            set_health(shared, true, None);
        }
        Verdict::Unhealthy(reason) => {
            *streak = streak.saturating_add(1);
            if should_report_unhealthy(*streak) {
                set_health(shared, false, Some(reason.to_string()));
            }
        }
    }
}

fn set_health(shared: &Shared, healthy: bool, reason: Option<String>) {
    let Ok(mut h) = shared.health.write() else {
        return;
    };
    let changed = h.healthy != healthy || h.reason != reason;
    if !changed {
        return;
    }
    if healthy {
        if !h.healthy {
            log::info!("health restored");
        }
        *h = Health::ok();
    } else {
        let why = reason.clone().unwrap_or_else(|| "unhealthy".into());
        log::warn!("unhealthy: {why}");
        *h = Health {
            healthy: false,
            reason,
        };
    }
}

/// One-shot apply (CLI `apply` / argv0 aliases).
pub fn apply_once(config_path: &Path) -> Result<Status> {
    let cfg = config::load(config_path)?;
    apply::apply(&cfg)
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
        Ok(ipc::Response::Status { healthy, .. }) => Ok(healthy),
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
    fn should_reselect_matrix() {
        let now = Instant::now() + Duration::from_secs(30);
        let retry = Duration::from_secs(15);
        let lost = now - Duration::from_secs(16);
        let recent = now - Duration::from_secs(5);
        assert!(!should_reselect(true, Some(lost), now, retry));
        assert!(!should_reselect(false, None, now, retry));
        assert!(!should_reselect(false, Some(recent), now, retry));
        assert!(should_reselect(false, Some(lost), now, retry));
        assert!(should_reselect(
            false,
            Some(lost),
            now,
            Duration::from_secs(10)
        ));
    }

    #[test]
    fn assess_matrix() {
        assert_eq!(assess(Mode::Gateway, false, false, false), Verdict::Healthy);
        assert_eq!(
            assess(Mode::Gateway, true, false, true),
            Verdict::Unhealthy("carrier but no IPv4")
        );
        assert_eq!(
            assess(Mode::Gateway, true, true, false),
            Verdict::Unhealthy("gateway without dnsmasq")
        );
        assert_eq!(assess(Mode::Gateway, true, true, true), Verdict::Healthy);
        assert_eq!(
            assess(Mode::Client, true, true, false),
            Verdict::Unhealthy("client without dhclient")
        );
        assert_eq!(assess(Mode::Client, true, true, true), Verdict::Healthy);
        assert_eq!(assess(Mode::Static, true, true, false), Verdict::Healthy);
    }

    #[test]
    fn heal_action_matrix() {
        assert_eq!(heal_action(Mode::Gateway, true), HealAction::None);
        assert_eq!(
            heal_action(Mode::Gateway, false),
            HealAction::RestartDnsmasq
        );
        assert_eq!(
            heal_action(Mode::Client, false),
            HealAction::RestartDhclient
        );
        assert_eq!(heal_action(Mode::Client, true), HealAction::None);
        assert_eq!(heal_action(Mode::Static, false), HealAction::None);
    }

    #[test]
    fn health_hysteresis_needs_three_fails() {
        assert!(!should_report_unhealthy(0));
        assert!(!should_report_unhealthy(1));
        assert!(!should_report_unhealthy(2));
        assert!(should_report_unhealthy(3));
        assert!(should_report_unhealthy(4));
    }
}
