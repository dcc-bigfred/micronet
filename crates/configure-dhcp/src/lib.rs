//! configure-dhcp — start dnsmasq when an event WiFi stack is detected on the LAN.

pub mod dhcp;
pub mod error;
pub mod run;
pub mod stack;
pub mod sticky;

pub use error::{Error, Result};
