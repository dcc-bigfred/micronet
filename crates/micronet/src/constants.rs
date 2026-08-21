//! Named bounds and well-known paths (CODING-GUIDELINES §1.3).

use std::time::Duration;

/// Maximum IPC JSON payload (bytes).
pub const MAX_IPC_FRAME_BYTES: usize = 1024 * 1024;
/// Concurrent Unix-socket clients.
pub const MAX_IPC_CLIENTS: usize = 32;
/// Config inotify debounce.
pub const CONFIG_DEBOUNCE: Duration = Duration::from_millis(300);
/// Default DHCPDISCOVER wait.
pub const DEFAULT_PROBE_TIMEOUT_SECS: u64 = 5;
/// ICMP: one echo, 2 s wait (same as former configure-ethernet).
pub const PING_COUNT: &str = "1";
pub const PING_TIMEOUT_SEC: &str = "2";
/// Wait for dhclient to assign an address.
pub const DHCP_CLIENT_WAIT: Duration = Duration::from_secs(5);

pub const IP_BIN: &str = "/sbin/ip";
pub const DHCLIENT_BIN: &str = "/sbin/dhclient";
pub const PING_BIN: &str = "/bin/ping";
pub const DNSMASQ_BIN: &str = "/usr/sbin/dnsmasq";

pub const DEFAULT_STICKY: &str = "7d";
pub const DEFAULT_RANGE_START: u8 = 50;
pub const DEFAULT_RANGE_END: u8 = 200;
pub const DEFAULT_STATIC_HOST: u8 = 252;
pub const REQUIRED_PREFIX: u8 = 24;

/// Linux `ARPHRD_ETHER`.
pub const ARPHRD_ETHER: u16 = 1;
