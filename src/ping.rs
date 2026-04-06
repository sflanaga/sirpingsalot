use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use socket2::{Domain, Protocol, SockAddr, Socket, Type};

use log::trace;

use std::io::Write;
use std::mem::MaybeUninit;

const ICMP_HEADER_SIZE: usize = 8;
const TOKEN_SIZE: usize = 0;
const ECHO_REQUEST_BUFFER_SIZE: usize = ICMP_HEADER_SIZE + TOKEN_SIZE;

type Token = [u8; TOKEN_SIZE];

struct ProtoTypeConsts {
    echo_request_type: u8,
    echo_request_code: u8,
    echo_reply_type: u8,
    echo_reply_code: u8,
}

const ICMPV4_CONST: ProtoTypeConsts = ProtoTypeConsts {
    echo_request_type: 8,
    echo_request_code: 0,
    echo_reply_type: 0,
    echo_reply_code: 0,
};

const ICMPV6_CONST: ProtoTypeConsts = ProtoTypeConsts {
    echo_request_type: 128,
    echo_request_code: 0,
    echo_reply_type: 129,
    echo_reply_code: 0,
};


pub struct Pinger {
    dest: SocketAddr,
    label: String,
    timeout: Duration,
    token: Token,
    socket: Socket,
    send_buffer: [u8; ECHO_REQUEST_BUFFER_SIZE],
    proto: ProtoTypeConsts,
    recv_buffer: [u8;1024],
}

impl Pinger {

    pub fn get_recv_buffer(&self, size: usize) -> &[u8] {
        &self.recv_buffer[..size]
    }

    pub fn new(addr: IpAddr, timeout: Duration, label: String) -> Result<Pinger> {
        let dest = SocketAddr::new(addr, 0);

        let (socket, proto) = if dest.is_ipv4() {
            (Socket::new(Domain::IPV4, Type::RAW, Some(Protocol::ICMPV4))
                 .with_context(|| format!("error from Socket::new ipv4: {}:{}", file!(), line!()))?,
             ICMPV4_CONST)
        } else {
            (Socket::new(Domain::IPV6, Type::RAW, Some(Protocol::ICMPV6))
                 .with_context(|| format!("error from Socket::new ipv6: {}:{}", file!(), line!()))?,
             ICMPV6_CONST)
        };

        Ok(Pinger {
            dest,
            label,
            timeout,
            token: [0u8; TOKEN_SIZE],
            socket: socket,
            send_buffer: [0u8; ECHO_REQUEST_BUFFER_SIZE],
            proto,
            recv_buffer: [0u8; 1024],
        })
    }

    fn encode(&mut self, ident: u16, seq: u16) -> Result<()> {
        self.send_buffer[0] = self.proto.echo_request_type;
        self.send_buffer[1] = self.proto.echo_request_code;

        self.send_buffer[4] = (ident >> 8) as u8;
        self.send_buffer[5] = ident as u8;
        self.send_buffer[6] = (seq >> 8) as u8;
        self.send_buffer[7] = seq as u8;
        if let Err(_) = (&mut self.send_buffer[ICMP_HEADER_SIZE..]).write(&self.token[..]) {
            return Err(anyhow!("invalid packet size"));
        }

        self.write_checksum();
        Ok(())
    }

    pub fn ping1(&mut self, ident: u16, seq: u16, ttl: u32) -> anyhow::Result<(usize, SockAddr), anyhow::Error> {
        self.encode(ident, seq)?;

        if self.dest.is_ipv4() {
            self.socket.set_ttl(ttl)
                .with_context(|| format!("error from set_ttl: {}:{}", file!(), line!()))?;
        } else {
            // TODO: why?
            //println!("not setting TTL for IPv6 as it currently fails for some reason");
        }

        trace!("{} sending buff: {:02X?}", self.label, &self.send_buffer);
        self.socket.send_to(&self.send_buffer, &self.dest.into())
            .with_context(|| format!("error from send_to: {}:{}", file!(), line!()))?;

        let deadline = Instant::now() + self.timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(anyhow!("timeout waiting for echo reply from {}", self.label));
            }

            self.socket.set_read_timeout(Some(remaining))
                .with_context(|| format!("error from set_read_timeout: {}:{}", file!(), line!()))?;

            let mut recv_buf = [MaybeUninit::<u8>::uninit(); 1024];
            let (ret_size, ret_sockaddr) = self.socket.recv_from(&mut recv_buf)
                .with_context(|| format!("error from recv_from: {}:{}", file!(), line!()))?;
            // SAFETY: recv_from initialized ret_size bytes
            unsafe {
                std::ptr::copy_nonoverlapping(
                    recv_buf.as_ptr() as *const u8,
                    self.recv_buffer.as_mut_ptr(),
                    ret_size,
                );
            }

            // Decode and check if this reply matches our ident — if not, discard and keep waiting
            match self.decode() {
                Ok((_type, _code, ret_ident, _seq)) if ret_ident == ident => {
                    trace!("{} ident sent: {} returned: {} seq sent: {} returned: {}", self.label, ident, ret_ident, seq, _seq);
                    return Ok((ret_size, ret_sockaddr));
                }
                Ok((_type, _code, ret_ident, _seq)) => {
                    trace!("{} discarding reply with ident {} (expected {})", self.label, ret_ident, ident);
                    continue;
                }
                Err(_) => {
                    trace!("{} discarding non-echo-reply packet", self.label);
                    continue;
                }
            }
        }
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
        if ret_type_ != self.proto.echo_reply_type || ret_code != self.proto.echo_reply_code {
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
