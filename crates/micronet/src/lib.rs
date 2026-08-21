//! Ethernet bring-up and DHCP gateway daemon for BigFred OS.

pub mod apply;
pub mod config;
pub mod constants;
pub mod daemon;
pub mod datadir;
pub mod dhcp;
pub mod error;
pub mod ipc;
pub mod net;
pub mod signals;
pub mod version;

pub use error::{Error, Result};
