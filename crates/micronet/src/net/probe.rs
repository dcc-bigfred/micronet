//! DHCPDISCOVER probe (no REQUEST). Foreign server → DHCPOFFER.

use std::net::{Ipv4Addr, SocketAddrV4};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use dhcproto::v4::{
    Decodable, Decoder, DhcpOption, Encodable, Encoder, Flags, HType, Message, MessageType, Opcode,
    OptionCode,
};
use socket2::{Domain, Protocol, SockAddr, Socket, Type};

use crate::error::{Error, Result};

const DHCP_CLIENT_PORT: u16 = 68;
const DHCP_SERVER_PORT: u16 = 67;
const DHCP_MAGIC_COOKIE: [u8; 4] = [0x63, 0x82, 0x53, 0x63];

/// Encode a DHCPDISCOVER (no I/O).
pub fn encode_discover(chaddr: &[u8; 6], xid: u32) -> Result<Vec<u8>> {
    let mut msg = Message::default();
    msg.set_opcode(Opcode::BootRequest);
    msg.set_htype(HType::Eth);
    msg.set_xid(xid);
    msg.set_flags(Flags::default().set_broadcast());
    msg.set_chaddr(chaddr);
    msg.opts_mut()
        .insert(DhcpOption::MessageType(MessageType::Discover));
    msg.opts_mut().insert(DhcpOption::ParameterRequestList(vec![
        OptionCode::SubnetMask,
        OptionCode::Router,
        OptionCode::DomainNameServer,
    ]));

    let mut buf = Vec::with_capacity(300);
    let mut enc = Encoder::new(&mut buf);
    msg.encode(&mut enc)
        .map_err(|e| Error::Other(format!("DHCP encode: {e}")))?;
    debug_assert!(buf.windows(4).any(|w| w == DHCP_MAGIC_COOKIE));
    Ok(buf)
}

/// True if `buf` is a DHCPOFFER.
#[must_use]
pub fn is_offer(buf: &[u8]) -> bool {
    let Ok(msg) = Message::decode(&mut Decoder::new(buf)) else {
        return false;
    };
    matches!(
        msg.opts().get(OptionCode::MessageType),
        Some(DhcpOption::MessageType(MessageType::Offer))
    )
}

/// Broadcast DHCPDISCOVER on `iface`; return true if any DHCPOFFER arrives.
pub fn probe_foreign_dhcp(iface: &str, mac: &[u8; 6], timeout: Duration) -> Result<bool> {
    let xid = xid_now();
    let pkt = encode_discover(mac, xid)?;
    let sock = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))
        .map_err(|e| Error::Other(format!("dhcp socket: {e}")))?;
    sock.set_reuse_address(true)
        .map_err(|e| Error::Other(format!("SO_REUSEADDR: {e}")))?;
    sock.set_broadcast(true)
        .map_err(|e| Error::Other(format!("SO_BROADCAST: {e}")))?;
    sock.set_read_timeout(Some(Duration::from_millis(250)))
        .map_err(|e| Error::Other(format!("SO_RCVTIMEO: {e}")))?;
    if let Err(e) = sock.bind_device(Some(iface.as_bytes())) {
        log::debug!("SO_BINDTODEVICE {iface}: {e}");
    }
    let bind = SockAddr::from(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, DHCP_CLIENT_PORT));
    sock.bind(&bind)
        .map_err(|e| Error::Other(format!("bind :{DHCP_CLIENT_PORT}: {e}")))?;

    let dest = SockAddr::from(SocketAddrV4::new(Ipv4Addr::BROADCAST, DHCP_SERVER_PORT));
    sock.send_to(&pkt, &dest)
        .map_err(|e| Error::Other(format!("DHCPDISCOVER send: {e}")))?;

    let udp = std::net::UdpSocket::from(sock);
    let deadline = Instant::now() + timeout;
    let mut buf = [0u8; 1500];
    while Instant::now() < deadline {
        match udp.recv_from(&mut buf) {
            Ok((n, _)) => {
                if is_offer(&buf[..n]) {
                    return Ok(true);
                }
            }
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(e) => {
                log::debug!("dhcp recv: {e}");
            }
        }
    }
    Ok(false)
}

fn xid_now() -> u32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(1)
}

/// Read MAC from sysfs `.../<iface>/address`.
pub fn read_mac(sys_class_net: &std::path::Path, iface: &str) -> Result<[u8; 6]> {
    let text = std::fs::read_to_string(sys_class_net.join(iface).join("address"))
        .map_err(|e| Error::io_at(sys_class_net.join(iface).join("address"), e))?;
    parse_mac(text.trim()).ok_or_else(|| Error::Other(format!("bad MAC on {iface}")))
}

fn parse_mac(s: &str) -> Option<[u8; 6]> {
    let mut out = [0u8; 6];
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 6 {
        return None;
    }
    for (i, p) in parts.iter().enumerate() {
        out[i] = u8::from_str_radix(p, 16).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn discover_has_cookie_and_type() {
        let mac = [0x02, 0x00, 0x00, 0x00, 0x00, 0x01];
        let buf = encode_discover(&mac, 0x1122_3344).unwrap();
        assert!(buf.windows(4).any(|w| w == DHCP_MAGIC_COOKIE));
        assert!(buf.windows(3).any(|w| w == [53, 1, 1]));
        assert!(!is_offer(&buf));
    }

    #[test]
    fn parse_mac_ok() {
        assert_eq!(
            parse_mac("aa:bb:cc:dd:ee:ff").unwrap(),
            [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]
        );
    }
}
