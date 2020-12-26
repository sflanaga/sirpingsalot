use std::net::{SocketAddr, IpAddr};
use std::time::Duration;
use anyhow::{anyhow,Context};
use rand::random;
use socket2::{Domain, Protocol, Socket, Type, SockAddr};

use crate::icmp::{EchoRequest, IcmpV4, IcmpV6, EchoReply, HEADER_SIZE as ICMP_HEADER_SIZE};
use crate::ipv4::IpV4Packet;

const TOKEN_SIZE: usize = 24;
const ECHO_REQUEST_BUFFER_SIZE: usize = ICMP_HEADER_SIZE + TOKEN_SIZE;
type Token = [u8; TOKEN_SIZE];

type ResultS<T> = std::result::Result<T, anyhow::Error>;

pub fn ping(addr: IpAddr, timeout: Option<Duration>, ttl: Option<u32>, ident: Option<u16>, seq_cnt: Option<u16>, payload: Option<&Token>)
    -> std::result::Result<(usize, SockAddr, u16, u16), anyhow::Error> {
    let timeout = match timeout {
        Some(timeout) => Some(timeout),
        None => Some(Duration::from_secs(4)),
    };

    let dest = SocketAddr::new(addr, 0);
    let mut buffer = [0; ECHO_REQUEST_BUFFER_SIZE];

    let default_payload: &Token = &random();

    let request = EchoRequest {
        ident: ident.unwrap_or(random()),
        seq_cnt: seq_cnt.unwrap_or(1),
        payload: payload.unwrap_or(default_payload),
    };

    let socket = if dest.is_ipv4() {
        if request.encode::<IcmpV4>(&mut buffer[..]).is_err() {
            return Err(anyhow!("error during encoding ipv4 packet"));
        }
        Socket::new(Domain::ipv4(), Type::raw(), Some(Protocol::icmpv4()))?
    } else {
        if request.encode::<IcmpV6>(&mut buffer[..]).is_err() {
            return Err(anyhow!("error during encoding ipv6 packet"));
        }
        Socket::new(Domain::ipv6(), Type::raw(), Some(Protocol::icmpv6()))?
    };

    socket.set_ttl(ttl.unwrap_or(64))?;

    socket.send_to(&mut buffer, &dest.into())?;

    socket.set_read_timeout(timeout)?;

    let mut buffer: [u8; 2048] = [0; 2048];
    let (size, sockaddr) = socket.recv_from(&mut buffer)?;

    let reply = if dest.is_ipv4() {
        let ipv4_packet = IpV4Packet::decode(&buffer)?;
        let reply = EchoReply::decode::<IcmpV4>(ipv4_packet.data)?;
        return Ok((size, sockaddr, reply.ident, reply.seq_cnt));
        // {
        //     Ok(reply) => reply,
        //     Err(_) => return Err(ErrorKind::InternalError.into()),
        // }
    } else {
        let reply = EchoReply::decode::<IcmpV6>(&buffer)?;
        return Ok((size, sockaddr, reply.ident, reply.seq_cnt));
    };
}
