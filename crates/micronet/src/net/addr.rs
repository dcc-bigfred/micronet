//! Host `.N` in a `/24` subnet.

use std::net::Ipv4Addr;

use ipnet::Ipv4Net;

use crate::constants::REQUIRED_PREFIX;
use crate::error::{Error, Result};

/// Addresses derived from gateway subnet + host octets.
pub struct IfaceAddrs {
    pub gateway: Ipv4Addr,
    pub static_host: Ipv4Addr,
    pub range_start: Ipv4Addr,
    pub range_end: Ipv4Addr,
}

/// Last-octet host in a `/24`.
pub fn host_in_slash24(net: Ipv4Net, host: u8) -> Result<Ipv4Addr> {
    if net.prefix_len() != REQUIRED_PREFIX {
        return Err(Error::Config(format!("subnet must be /{REQUIRED_PREFIX}")));
    }
    let o = net.network().octets();
    Ok(Ipv4Addr::new(o[0], o[1], o[2], host))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn host_252() {
        let net: Ipv4Net = "10.0.10.0/24".parse().unwrap();
        assert_eq!(
            host_in_slash24(net, 252).unwrap(),
            Ipv4Addr::new(10, 0, 10, 252)
        );
    }
}
