use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use rand::random;
use socket2::{Domain, Protocol, SockAddr, Socket, Type};

use log::{debug, error, info, trace, warn};

use crate::icmp::{EchoReply, EchoRequest, HEADER_SIZE as ICMP_HEADER_SIZE, IcmpV4, IcmpV6};
use crate::ipv4::IpV4Packet;
use std::io::Write;

const TOKEN_SIZE: usize = 0;
const ECHO_REQUEST_BUFFER_SIZE: usize = ICMP_HEADER_SIZE + TOKEN_SIZE;

type Token = [u8; TOKEN_SIZE];

struct ProtoTypeConsts {
    ECHO_REQUEST_TYPE: u8,
    ECHO_REQUEST_CODE: u8,
    ECHO_REPLY_TYPE: u8,
    ECHO_REPLY_CODE: u8,
    HEADER_SIZE: usize,
}

const ICMPV4_CONST: ProtoTypeConsts = ProtoTypeConsts {
    ECHO_REQUEST_TYPE: 8,
    ECHO_REQUEST_CODE: 0,
    ECHO_REPLY_TYPE: 0,
    ECHO_REPLY_CODE: 0,
    HEADER_SIZE: 8,
};

const ICMPV6_CONST: ProtoTypeConsts = ProtoTypeConsts {
    ECHO_REQUEST_TYPE: 128,
    ECHO_REQUEST_CODE: 0,
    ECHO_REPLY_TYPE: 129,
    ECHO_REPLY_CODE: 0,
    HEADER_SIZE: 8,
};


pub struct Pinger {
    dest: SocketAddr,
    timeout: Duration,
    token: Token,
    socket: Socket,
    send_buffer: [u8; ECHO_REQUEST_BUFFER_SIZE],
    proto: ProtoTypeConsts,
    recv_buffer: [u8;1024],
}

impl Pinger {

    pub fn get_send_buffer(&self, size: usize) -> &[u8] {
        &self.send_buffer[..size]
    }

    pub fn get_recv_buffer(&self, size: usize) -> &[u8] {
        &self.recv_buffer[..size]
    }

    pub fn new(addr: IpAddr, timeout: Duration) -> Result<Pinger> {
        let dest = SocketAddr::new(addr, 0);
        let default_payload: &Token = &random();

        let (socket, proto) = if dest.is_ipv4() {
            (Socket::new(Domain::ipv4(), Type::raw(), Some(Protocol::icmpv4()))
                 .with_context(|| format!("error from Socket::new ipv4: {}:{}", file!(), line!()))?,
             ICMPV4_CONST)
        } else {
            (Socket::new(Domain::ipv6(), Type::raw(), Some(Protocol::icmpv6()))
                 .with_context(|| format!("error from Socket::new ipv6: {}:{}", file!(), line!()))?,
             ICMPV6_CONST)
        };

        Ok(Pinger {
            dest,
            timeout,
            token: [0u8; TOKEN_SIZE],
            socket: socket,
            send_buffer: [0u8; ECHO_REQUEST_BUFFER_SIZE],
            proto,
            recv_buffer: [0u8; 1024],
        })
    }

    fn encode(&mut self, ident: u16, seq: u16) -> Result<()> {
        pub const HEADER_SIZE: usize = 8;
        self.send_buffer[0] = self.proto.ECHO_REQUEST_TYPE;
        self.send_buffer[1] = self.proto.ECHO_REQUEST_CODE;

        self.send_buffer[4] = (ident >> 8) as u8;
        self.send_buffer[5] = ident as u8;
        self.send_buffer[6] = (seq >> 8) as u8;
        self.send_buffer[7] = seq as u8;
        if let Err(_) = (&mut self.send_buffer[HEADER_SIZE..]).write(&self.token[..]) {
            return Err(anyhow!("invalid packet size"));
        }

        self.write_checksum();
        Ok(())
    }

    pub fn ping1(&mut self, ident: u16, seq: u16, ttl: u32) -> anyhow::Result<(usize, SockAddr), anyhow::Error> {
        self.encode(ident, seq);

        if self.dest.is_ipv4() {
            self.socket.set_ttl(ttl)
                .with_context(|| format!("error from set_ttl: {}:{}", file!(), line!()))?;
        } else {
            // TODO: why?
            //println!("not setting TTL for IPv6 as it currently fails for some reason");
        }

        trace!("{} sending buff: {:02X?}", self.dest.ip(), &self.send_buffer);
        self.socket.send_to(&self.send_buffer, &self.dest.into())
            .with_context(|| format!("error from send_to: {}:{}", file!(), line!()))?;

        self.socket.set_read_timeout(Some(self.timeout))
            .with_context(|| format!("error from set_read_timeout: {}:{}", file!(), line!()))?;

        let (ret_size, ret_sockaddr) = self.socket.recv_from(&mut self.recv_buffer[0..])
            .with_context(|| format!("error from recv_from: {}:{}", file!(), line!()))?;

        self.decode();
        Ok((ret_size, ret_sockaddr))
    }

    pub fn decode(&mut self) -> Result<(u8,u8,u16,u16)> {
        let mut header_size = 0usize;
        if self.dest.is_ipv4() {
            let byte0 = self.recv_buffer[0];
            let version = (byte0 & 0xf0) >> 4;
            header_size = 4 * ((byte0 & 0x0f) as usize);
            let ttl = self.recv_buffer[8];
            trace!("ipv4 header: v {} header_size: {} ttl: {}", version, header_size, ttl);
        }

        let icmp_data = &self.recv_buffer[header_size..];

        let ret_type_ = icmp_data[0];
        let ret_code = icmp_data[1];
        if ret_type_ != self.proto.ECHO_REPLY_TYPE && ret_code != self.proto.ECHO_REPLY_CODE {
            return Err(anyhow!("invalid packet"));
        }

        let ret_ident = (u16::from(icmp_data[4]) << 8) + u16::from(icmp_data[5]);
        let ret_seq = (u16::from(icmp_data[6]) << 8) + u16::from(icmp_data[7]);
        Ok((ret_type_, ret_code, ret_ident, ret_seq))
    }

    fn write_checksum(&mut self) {
        // pre clear the prior checksum
        self.send_buffer[2] = 0;
        self.send_buffer[3] = 0;
        trace!("before checksum: {:02X?}", self.send_buffer);

        let mut sum = 0u32;
        for word in self.send_buffer.chunks(2) {
            let mut part = u16::from(word[0]) << 8;
            if word.len() > 1 {
                part += u16::from(word[1]);
            }
            sum = sum.wrapping_add(u32::from(part));
        }

        while (sum >> 16) > 0 {
            sum = (sum & 0xffff) + (sum >> 16);
        }

        let sum = !sum as u16;

        self.send_buffer[2] = (sum >> 8) as u8;
        self.send_buffer[3] = (sum & 0xff) as u8;

        trace!("after checksum: {:02X?}", self.send_buffer);

    }
}

pub fn ping(addr: IpAddr, timeout: Option<Duration>, ttl: Option<u32>,
            ident: Option<u16>, seq_cnt: Option<u16>, payload: Option<&Token>, verbose: usize,
            msg_buf: &mut String, record_raw_bits: bool)
            -> Result<(usize, SockAddr, u8, u8, u16, u16, u8), anyhow::Error> {
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
            .with_context(|| format!("error from Socket::new ipv4: {}:{}", file!(), line!()))?
    } else {
        if verbose > 2 { println!("built echo request - now encoding ipv6 {},{}", file!(), line!()); }
        if let Err(e) = request.encode::<IcmpV6>(&mut buffer[..]) {
            return Err(anyhow!("error during encoding ipv6 packet: error {}", e));
        }
        Socket::new(Domain::ipv6(), Type::raw(), Some(Protocol::icmpv6()))
            .with_context(|| format!("error from Socket::new ipv6: {}:{}", file!(), line!()))?
    };
    if dest.is_ipv4() {
        if verbose > 2 { println!("encoded - now set ttl {},{}", file!(), line!()); }
        socket.set_ttl(ttl.unwrap_or(64))
            .with_context(|| format!("error from set_ttl: {}:{}", file!(), line!()))?;
    } else {
        // TODO: why?
        //println!("not setting TTL for IPv6 as it currently fails for some reason");
    }

    if verbose > 2 { println!("set ttl worked - now sending {},{}", file!(), line!()); }
    socket.send_to(&mut buffer, &dest.into())
        .with_context(|| format!("error from send_to: {}:{}", file!(), line!()))?;

    if verbose > 2 { println!("sent - now waiting {},{}", file!(), line!()); }
    socket.set_read_timeout(timeout)
        .with_context(|| format!("error from set_read_timeout: {}:{}", file!(), line!()))?;

    if verbose > 2 { println!("wait done - now getting response {},{}", file!(), line!()); }
    let mut buffer: [u8; 2048] = [0; 2048];
    let (size, sockaddr) = socket.recv_from(&mut buffer)
        .with_context(|| format!("error from recv_from: {}:{}", file!(), line!()))?;

    // if record_raw_bits {
    //     msg_buf.clear();
    //     write!(msg_buf, "ip: {} pkt size: {} raw data: {:02X?}", addr, size, &buffer[0..size]);
    // }
    //
    let reply = if dest.is_ipv4() {
        if verbose > 2 { println!("response received - now decoding ipv4 {},{}", file!(), line!()); }
        let ipv4_packet = IpV4Packet::decode(&buffer)?;
        let reply = EchoReply::decode::<IcmpV4>(ipv4_packet.data)
            .with_context(|| format!("error from EchoReply::decode ipv4: {}:{}", file!(), line!()))?;
        return Ok((size, sockaddr, reply.type_, reply.code, reply.ident, reply.seq_cnt, ipv4_packet.ttl));
        // {
        //     Ok(reply) => reply,
        //     Err(_) => return Err(ErrorKind::InternalError.into()),
        // }
    } else {
        if verbose > 2 { println!("response received - now decoding ipv6 {},{}", file!(), line!()); }
        let reply = EchoReply::decode::<IcmpV6>(&buffer)
            .with_context(|| format!("error from EchoReply::decode ipv4: {}:{}", file!(), line!()))?;
        return Ok((size, sockaddr, reply.type_, reply.code, reply.ident, reply.seq_cnt, 0));
    };
}
