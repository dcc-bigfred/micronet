//! DHCP helpers: conf, leases, process control.

pub mod conf;
pub mod leases;
pub mod run;

pub use conf::{
    ensure_base_conf, ensure_ethernet_primary, merge_reservations, render_base_conf,
    write_reservations, DhcpDefaults, DEFAULT_ETHERNET_CONF, DEFAULT_GATEWAY,
};
pub use leases::{parse_arp_file, parse_leases_file, LeaseEntry};
pub use run::{
    dnsmasq_exists, dnsmasq_running, ensure_gateway_addr, first_ethernet_iface, iface_has_ipv4,
    sighup_dnsmasq, start_dnsmasq,
};
