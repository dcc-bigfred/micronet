//! dnsmasq DHCP server (gateway mode only).

pub mod conf;
pub mod run;

pub use conf::render_conf;
pub use run::{is_running, reload_or_restart, start, stop};
