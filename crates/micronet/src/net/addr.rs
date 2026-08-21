//! Host `.N` in a `/24` subnet and `ip` CIDR parsing.

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

/// First IPv4 CIDR from `ip -4 -o addr show` stdout. Prefers non-link-local.
#[must_use]
pub fn parse_first_inet_cidr(ip_stdout: &str) -> Option<String> {
    let mut found: Vec<String> = Vec::new();
    for line in ip_stdout.lines() {
        let Some(rest) = line.split("inet ").nth(1) else {
            continue;
        };
        let Some(token) = rest.split_whitespace().next() else {
            continue;
        };
        if token.parse::<Ipv4Net>().is_ok() {
            found.push(token.to_string());
        }
    }
    let non_ll = found.iter().find(|s| !s.starts_with("169.254.")).cloned();
    non_ll.or_else(|| found.into_iter().next())
}

/// IPv4 address from a `a.b.c.d/nn` string.
#[must_use]
pub fn cidr_ipv4(cidr: &str) -> Option<Ipv4Addr> {
    cidr.split('/').next().and_then(|s| s.parse().ok())
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

    #[test]
    fn parse_first_inet_cidr_variants() {
        assert_eq!(
            parse_first_inet_cidr(
                "2: eth0    inet 10.0.10.50/24 brd 10.0.10.255 scope global eth0\n"
            )
            .as_deref(),
            Some("10.0.10.50/24")
        );
        assert!(parse_first_inet_cidr("2: eth0    inet6 fe80::1/64\n").is_none());
        let two = "\
2: eth0    inet 169.254.1.1/16 scope link eth0
2: eth0    inet 10.0.10.1/24 brd 10.0.10.255 scope global eth0
";
        assert_eq!(parse_first_inet_cidr(two).as_deref(), Some("10.0.10.1/24"));
        assert!(parse_first_inet_cidr("").is_none());
    }
}
