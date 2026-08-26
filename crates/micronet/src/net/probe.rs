//! DHCPDISCOVER probe (no REQUEST). Foreign server → DHCPOFFER.

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use dhcproto::v4::{
    Decodable, Decoder, DhcpOption, Encodable, Encoder, Flags, HType, Message, MessageType, Opcode,
    OptionCode,
};
use socket2::{Domain, Protocol, SockAddr, Socket, Type};

use crate::constants::DHCP_PROBE_RECV_TIMEOUT;
use crate::error::{Error, Result};

const DHCP_CLIENT_PORT: u16 = 68;
const DHCP_SERVER_PORT: u16 = 67;
const DHCP_MAGIC_COOKIE: [u8; 4] = [0x63, 0x82, 0x53, 0x63];

/// Fields we need from a validated DHCPOFFER.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OfferView {
    pub server_id: Option<Ipv4Addr>,
}

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
        .map_err(|e| Error::DhcpProbe(format!("DHCP encode: {e}")))?;
    debug_assert!(buf.windows(4).any(|w| w == DHCP_MAGIC_COOKIE));
    Ok(buf)
}

/// Decode a DHCPOFFER that matches our DISCOVER (`xid` + `chaddr`).
#[must_use]
pub fn decode_matching_offer(buf: &[u8], xid: u32, chaddr: &[u8; 6]) -> Option<OfferView> {
    let msg = Message::decode(&mut Decoder::new(buf)).ok()?;
    if msg.opcode() != Opcode::BootReply {
        return None;
    }
    if msg.htype() != HType::Eth {
        return None;
    }
    if msg.hlen() != 6 {
        return None;
    }
    if msg.xid() != xid {
        return None;
    }
    let got = msg.chaddr();
    if got.len() < 6 || got[..6] != chaddr[..] {
        return None;
    }
    match msg.opts().get(OptionCode::MessageType) {
        Some(DhcpOption::MessageType(MessageType::Offer)) => {}
        _ => return None,
    }
    let server_id = match msg.opts().get(OptionCode::ServerIdentifier) {
        Some(DhcpOption::ServerIdentifier(ip)) => Some(*ip),
        _ => None,
    };
    Some(OfferView { server_id })
}

/// Broadcast DHCPDISCOVER on `iface`; true if a DHCPOFFER arrives.
///
/// Own dnsmasq is stopped before this probe, so any matching offer is foreign.
/// Bind/send/`SO_BINDTODEVICE` failures are errors (fail closed). Timeout
/// with no valid offer is `Ok(false)`.
pub fn probe_foreign_dhcp(iface: &str, mac: &[u8; 6], timeout: Duration) -> Result<bool> {
    let xid = xid_now();
    let pkt = encode_discover(mac, xid)?;
    let sock = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))
        .map_err(|e| Error::DhcpProbe(format!("dhcp socket: {e}")))?;
    sock.set_reuse_address(true)
        .map_err(|e| Error::DhcpProbe(format!("SO_REUSEADDR: {e}")))?;
    sock.set_broadcast(true)
        .map_err(|e| Error::DhcpProbe(format!("SO_BROADCAST: {e}")))?;
    sock.set_read_timeout(Some(DHCP_PROBE_RECV_TIMEOUT))
        .map_err(|e| Error::DhcpProbe(format!("SO_RCVTIMEO: {e}")))?;
    sock.bind_device(Some(iface.as_bytes()))
        .map_err(|e| Error::DhcpProbe(format!("SO_BINDTODEVICE {iface}: {e}")))?;
    let bind = SockAddr::from(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, DHCP_CLIENT_PORT));
    sock.bind(&bind)
        .map_err(|e| Error::DhcpProbe(format!("bind :{DHCP_CLIENT_PORT}: {e}")))?;

    let dest = SockAddr::from(SocketAddrV4::new(Ipv4Addr::BROADCAST, DHCP_SERVER_PORT));
    sock.send_to(&pkt, &dest)
        .map_err(|e| Error::DhcpProbe(format!("DHCPDISCOVER send: {e}")))?;

    let udp = std::net::UdpSocket::from(sock);
    let deadline = Instant::now() + timeout;
    let mut buf = [0u8; 1500];
    while Instant::now() < deadline {
        match udp.recv_from(&mut buf) {
            Ok((n, src)) => {
                if let Some(offer) = decode_matching_offer(&buf[..n], xid, mac) {
                    if offer_identity(offer.server_id, udp_src_v4(src)).is_some() {
                        return Ok(true);
                    }
                }
            }
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(e) => {
                return Err(Error::DhcpProbe(format!("dhcp recv: {e}")));
            }
        }
    }
    Ok(false)
}

fn udp_src_v4(src: SocketAddr) -> Ipv4Addr {
    match src {
        SocketAddr::V4(v) => *v.ip(),
        SocketAddr::V6(_) => Ipv4Addr::UNSPECIFIED,
    }
}

/// Server-id, or a non-zero UDP source. `0.0.0.0` with no option 54 cannot be distinguished.
fn offer_identity(server_id: Option<Ipv4Addr>, udp_src: Ipv4Addr) -> Option<Ipv4Addr> {
    if let Some(id) = server_id {
        return Some(id);
    }
    if udp_src != Ipv4Addr::UNSPECIFIED {
        return Some(udp_src);
    }
    None
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
    parse_mac(text.trim()).ok_or_else(|| Error::DhcpProbe(format!("bad MAC on {iface}")))
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

    fn encode_offer(
        chaddr: &[u8; 6],
        xid: u32,
        server_id: Option<Ipv4Addr>,
        opcode: Opcode,
        htype: HType,
        msg_type: MessageType,
    ) -> Vec<u8> {
        let mut msg = Message::default();
        msg.set_opcode(opcode);
        msg.set_htype(htype);
        msg.set_xid(xid);
        msg.set_chaddr(chaddr);
        msg.opts_mut().insert(DhcpOption::MessageType(msg_type));
        if let Some(id) = server_id {
            msg.opts_mut().insert(DhcpOption::ServerIdentifier(id));
        }
        let mut buf = Vec::with_capacity(300);
        let mut enc = Encoder::new(&mut buf);
        msg.encode(&mut enc).unwrap();
        buf
    }

    #[test]
    fn discover_has_cookie_and_type() {
        let mac = [0x02, 0x00, 0x00, 0x00, 0x00, 0x01];
        let buf = encode_discover(&mac, 0x1122_3344).unwrap();
        assert!(buf.windows(4).any(|w| w == DHCP_MAGIC_COOKIE));
        assert!(buf.windows(3).any(|w| w == [53, 1, 1]));
        assert!(decode_matching_offer(&buf, 0x1122_3344, &mac).is_none());
    }

    #[test]
    fn matching_offer_is_accepted() {
        let mac = [0x02, 0x00, 0x00, 0x00, 0x00, 0x01];
        let xid = 0xAABB_CCDD;
        let buf = encode_offer(
            &mac,
            xid,
            Some(Ipv4Addr::new(8, 8, 8, 8)),
            Opcode::BootReply,
            HType::Eth,
            MessageType::Offer,
        );
        let view = decode_matching_offer(&buf, xid, &mac).unwrap();
        assert_eq!(view.server_id, Some(Ipv4Addr::new(8, 8, 8, 8)));
    }

    #[test]
    fn wrong_xid_chaddr_opcode_htype_ack_rejected() {
        let mac = [0x02, 0x00, 0x00, 0x00, 0x00, 0x01];
        let other = [0x02, 0x00, 0x00, 0x00, 0x00, 0x02];
        let xid = 1u32;
        let good = || {
            encode_offer(
                &mac,
                xid,
                None,
                Opcode::BootReply,
                HType::Eth,
                MessageType::Offer,
            )
        };
        assert!(decode_matching_offer(&good(), 2, &mac).is_none());
        assert!(decode_matching_offer(
            &encode_offer(
                &other,
                xid,
                None,
                Opcode::BootReply,
                HType::Eth,
                MessageType::Offer,
            ),
            xid,
            &mac
        )
        .is_none());
        assert!(decode_matching_offer(
            &encode_offer(
                &mac,
                xid,
                None,
                Opcode::BootRequest,
                HType::Eth,
                MessageType::Offer,
            ),
            xid,
            &mac
        )
        .is_none());
        assert!(decode_matching_offer(
            &encode_offer(
                &mac,
                xid,
                None,
                Opcode::BootReply,
                HType::Eth,
                MessageType::Ack,
            ),
            xid,
            &mac
        )
        .is_none());
    }

    #[test]
    fn offer_identity_prefers_server_id() {
        let foreign = Ipv4Addr::new(8, 8, 8, 8);
        assert!(offer_identity(None, Ipv4Addr::UNSPECIFIED).is_none());
        assert_eq!(offer_identity(None, foreign), Some(foreign));
        assert_eq!(
            offer_identity(Some(foreign), Ipv4Addr::UNSPECIFIED),
            Some(foreign)
        );
    }

    #[test]
    fn truncated_buffer_is_none() {
        let mac = [0u8; 6];
        assert!(decode_matching_offer(&[0, 1, 2], 1, &mac).is_none());
    }

    #[test]
    fn parse_mac_ok() {
        assert_eq!(
            parse_mac("aa:bb:cc:dd:ee:ff").unwrap(),
            [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]
        );
    }
}
