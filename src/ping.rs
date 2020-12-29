use std::net::{SocketAddr, IpAddr};
use std::time::Duration;
use anyhow::{anyhow, Context, Result};
use rand::random;
use socket2::{Domain, Protocol, Socket, Type, SockAddr};

use crate::icmp::{EchoRequest, IcmpV4, IcmpV6, EchoReply, HEADER_SIZE as ICMP_HEADER_SIZE};
use crate::ipv4::IpV4Packet;

const TOKEN_SIZE: usize = 24;
const ECHO_REQUEST_BUFFER_SIZE: usize = ICMP_HEADER_SIZE + TOKEN_SIZE;

type Token = [u8; TOKEN_SIZE];

pub fn ping(addr: IpAddr, timeout: Option<Duration>, ttl: Option<u32>, ident: Option<u16>, seq_cnt: Option<u16>, payload: Option<&Token>, verbose: usize)
            -> Result<(usize, SockAddr, u16, u16, u8), anyhow::Error> {
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
        if verbose > 2 { println!("built echo request - now encoding ipv4 {},{}", file!(), line!()); }
        if let Err(e) = request.encode::<IcmpV4>(&mut buffer[..]) {
            return Err(anyhow!("error during encoding ipv4 packet: error {}", e));
        }
        Socket::new(Domain::ipv4(), Type::raw(), Some(Protocol::icmpv4()))
            .with_context(|| format!("error from Socket::new ipv4: {}:{}",file!(), line!()))?
    } else {
        if verbose > 2 { println!("built echo request - now encoding ipv6 {},{}", file!(), line!()); }
        if let Err(e)  = request.encode::<IcmpV6>(&mut buffer[..]) {
            return Err(anyhow!("error during encoding ipv6 packet: error {}", e));
        }
        Socket::new(Domain::ipv6(), Type::raw(), Some(Protocol::icmpv6()))
            .with_context(|| format!("error from Socket::new ipv6: {}:{}",file!(), line!()))?
    };
    if dest.is_ipv4() {
        if verbose > 2 { println!("encoded - now set ttl {},{}", file!(), line!()); }
        socket.set_ttl(ttl.unwrap_or(64))
            .with_context(|| format!("error from set_ttl: {}:{}",file!(), line!()))?;
    } else {
        // TODO: why?
        //println!("not setting TTL for IPv6 as it currently fails for some reason");
    }

    if verbose > 2 { println!("set ttl worked - now sending {},{}", file!(), line!()); }
    socket.send_to(&mut buffer, &dest.into())
        .with_context(|| format!("error from send_to: {}:{}",file!(), line!()))?;

    if verbose > 2 { println!("sent - now waiting {},{}", file!(), line!()); }
    socket.set_read_timeout(timeout)
        .with_context(|| format!("error from set_read_timeout: {}:{}",file!(), line!()))?;

    if verbose > 2 { println!("wait done - now getting response {},{}", file!(), line!()); }
    let mut buffer: [u8; 2048] = [0; 2048];
    let (size, sockaddr) = socket.recv_from(&mut buffer)
        .with_context(|| format!("error from recv_from: {}:{}",file!(), line!()))?;

    if verbose > 2 {
        println!("ip: {} pkt size: {} data: {:02X?}", addr, size, &buffer[0..size]);
    }

    let reply = if dest.is_ipv4() {
        if verbose > 2 { println!("response received - now decoding ipv4 {},{}", file!(), line!()); }
        let ipv4_packet = IpV4Packet::decode(&buffer)?;
        let reply = EchoReply::decode::<IcmpV4>(ipv4_packet.data)
            .with_context(|| format!("error from EchoReply::decode ipv4: {}:{}",file!(), line!()))?;
        return Ok((size, sockaddr, reply.ident, reply.seq_cnt, ipv4_packet.ttl));
        // {
        //     Ok(reply) => reply,
        //     Err(_) => return Err(ErrorKind::InternalError.into()),
        // }
    } else {
        if verbose > 2 { println!("response received - now decoding ipv6 {},{}", file!(), line!()); }
        let reply = EchoReply::decode::<IcmpV6>(&buffer)
            .with_context(|| format!("error from EchoReply::decode ipv4: {}:{}",file!(), line!()))?;
        return Ok((size, sockaddr, reply.ident, reply.seq_cnt, 0));
    };
}
