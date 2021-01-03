#![allow(unused_imports, unused_variables, unused_mut, unused_parens, unreachable_code)]

use socket2::{Domain, Protocol, SockAddr, Socket, Type};
use anyhow::{Context, anyhow};
use std::time::{Duration, SystemTime, Instant};
use humantime::format_rfc3339_millis;
use structopt::StructOpt;
use log::{debug, error, info, trace, warn};

mod cli;
mod icmp;
mod util;
mod stop;
mod stats;

use icmp::*;
use std::net::{SocketAddr, SocketAddrV4, Shutdown, IpAddr};
use std::io::Write;

use cli::*;
use stats::*;
use util::*;
use crate::stop::Stop;
use std::sync::{Arc, Mutex};
use std::collections::{HashMap, HashSet};

fn main() {
    match run() {
        Ok(_) => {}
        Err(e) => { eprintln!("error: {:#?}", e) }
    }
}


fn run() -> Result<(), anyhow::Error> {
    let cfg: Config = Config::from_args();
    init_log(cfg.log_level);

    error!("starting...");

    let mut stop = Stop::new();

    let mut tracker = Tracks::new(&cfg)?;

    let recv4 = {
        let mut tracker = tracker.clone();
        std::thread::Builder::new()
            .name(String::from("recv4"))
            .spawn(move || listen_icmp(&ICMPV4_CONST, tracker))?
    };

    let recv6 = {
        let mut tracker = tracker.clone();
        std::thread::Builder::new()
            .name(String::from("recv6"))
            .spawn(move || listen_icmp(&ICMPV6_CONST, tracker))?
    };

    // let mut tracks = vec![];
    // for (no, ip) in cfg.ips.iter().enumerate() {
    //     let mut stats = Arc::new(Stats::new());
    //     tracks.push(stats.clone());
    // }

    let send = {
        let (mut tracker, mut stop) = (tracker.clone(), stop.clone());
        let cfg = cfg.clone();
        std::thread::Builder::new()
            .name(String::from("send"))
            .spawn(move || send_icmp(cfg, tracker, stop))?
    };

    if cfg.stat_interval.as_millis() > 0 {
        debug!("starting stats thread");
        let (mut stop, stats_interval) = (stop.clone(), cfg.stat_interval.clone());
        let tracker_h = std::thread::Builder::new()
            .name(String::from("stats"))
            .spawn(move || stats_thread(tracker, stop, stats_interval))?;
    } else {
        info!("no stats tracking started");
        while !stop.sleep(Duration::from_secs(60)) {}
        std::process::exit(0);
    }

    recv4.join();

    Ok(())
}

fn send_icmp(cfg: Config, mut tracker: Tracks, mut stop: Stop) {
    match _send_icmp(cfg, tracker, stop) {
        Ok(_) => {}
        Err(e) => { panic!("sender - thread death - error: {:#?}", e) }
    }
}

fn _send_icmp(cfg: Config, mut tracker: Tracks, mut stop: Stop) -> Result<(), anyhow::Error> {
    let soc4 = Socket::new(Domain::ipv4(), Type::raw(), Some(Protocol::icmpv4()))
        .with_context(|| format!("error from Socket::new ipv4: {}:{}", file!(), line!()))?;
    soc4.set_ttl(255);

    let soc6 = Socket::new(Domain::ipv6(), Type::raw(), Some(Protocol::icmpv6()))
        .with_context(|| format!("error from Socket::new ipv6: {}:{}", file!(), line!()))?;

    let mut buffer = [0u8; 32];
    let mut buf = [0u8; 32];

    let mut seq = 11000u16;
    let ident = 22000u16;

    // pre compute to save time in actual loop?
    let mut v = vec![];
    for (n, addr) in cfg.ips.iter().enumerate() {
        let n = n as u16;
        v.push(UpdateSendIteration {
            ident: ident + n,
            ip: addr.ip,
            sa: SockAddr::from(SocketAddr::new(addr.ip, 0)),
            now: Instant::now(),
        });
    }

    loop {
        for i in v.iter_mut() {
            if i.ip.is_ipv4() {
                encode(&ICMPV4_CONST, &mut buf, i.ident, seq as u16);
                trace!("sending... {:?} seq: {}", &i.sa, seq);
                soc4.send_to(&buf, &i.sa)
                    .with_context(|| format!("error in send_to4: {}:{}", file!(), line!()))?;
            } else {
                encode(&ICMPV6_CONST, &mut buf, i.ident, seq as u16);
                trace!("sending... {:?} seq: {}", &i.sa, seq);
                soc6.send_to(&buf, &i.sa)
                    .with_context(|| format!("error in send_to6: {}:{}", file!(), line!()))?;
            }
            i.now = Instant::now();
        }
        tracker.update_for_send_bulk(&v, seq);

        if stop.sleep(cfg.interval) {
            std::process::exit(1);
        }
        //std::thread::sleep(Duration::from_secs(2));
        seq = seq.wrapping_add(1);
    }

    Ok(())
}

fn listen_icmp(proto: &ProtoTypeConsts, mut tracking: Tracks) {
    match _listen_icmp(proto, tracking) {
        Ok(_) => {}
        Err(e) => { panic!("listener for icmp {} - thread death - error: {:#?}", &proto.ver, e) }
    }
}

fn _listen_icmp(proto: &ProtoTypeConsts, mut tracking: Tracks) -> Result<(), anyhow::Error> {
    let mut soc = if proto.isV4 {
        let soc = Socket::new(Domain::ipv4(), Type::raw(), Some(Protocol::icmpv4()))
            .with_context(|| format!("error from Socket::new ipv4: {}:{}", file!(), line!()))?;
        soc.set_ttl(255);
        soc
    } else {
        Socket::new(Domain::ipv6(), Type::raw(), Some(Protocol::icmpv6()))
            .with_context(|| format!("error from Socket::new ipv4: {}:{}", file!(), line!()))?
    };
    soc.set_read_timeout(Some(Duration::from_secs(60)))?;

    let mut buffer = [0u8; 1024];

    loop {
        trace!("waiting...");

        let res = soc.recv_from(&mut buffer);
        match res {
            Err(e) => {
                match e.kind() {
                    std::io::ErrorKind::TimedOut => debug!("[{}]  ticking against timeout", format_rfc3339_millis(SystemTime::now())),
                    _ => panic!("thread death - error: {:#?}", e),
                }
            }
            Ok((size, ret_addr)) => {
                let r = IcmpEchoReply::decode(&buffer[0..], &proto).unwrap();
                let ip = ret_addr.as_std().unwrap().ip();
                let ver = if ip.is_ipv4() {
                    "V4"
                } else if ip.is_ipv6(){
                    "V6"
                } else {
                    "V?"
                };
                if let Some(r) = r {
                    let now = Instant::now();
                    if !tracking.update_for_recv(ip, now, r.ident, r.seq) {
                        trace!("{} PACKET from unexpected ip: {} size: {}  raw: {:02X?}\n reply: {:?}", ver, ip, size, &buffer[..size], &r);
                    } else {
                        trace!("{} PACKET size: {}  raw: {:02X?}\n reply: {:?}", ver, size, &buffer[..size], &r);
                    }
                } else {
                    trace!("bad reply code from {}", ip);
                    trace!("{} PACKET size: {}  raw: {:02X?}\n reply: NONE", ver, size, &buffer[..size]);
                }
            }
        }
    }

    Ok(())
}


fn encode(proto: &ProtoTypeConsts, buff: &mut [u8], ident: u16, seq: u16) -> Result<(), anyhow::Error> {
    pub const HEADER_SIZE: usize = 8;
    buff[0] = proto.ECHO_REQUEST_TYPE;
    buff[1] = proto.ECHO_REQUEST_CODE;

    buff[4] = (ident >> 8) as u8;
    buff[5] = ident as u8;
    buff[6] = (seq >> 8) as u8;
    buff[7] = seq as u8;

    write_checksum(buff);
    Ok(())
}


struct ProtoTypeConsts {
    isV4: bool,
    ver: &'static str,
    ECHO_REQUEST_TYPE: u8,
    ECHO_REQUEST_CODE: u8,
    ECHO_REPLY_TYPE: u8,
    ECHO_REPLY_CODE: u8,
    HEADER_SIZE: usize,
}

const ICMPV4_CONST: ProtoTypeConsts = ProtoTypeConsts {
    isV4: true,
    ver: "V4",
    ECHO_REQUEST_TYPE: 8,
    ECHO_REQUEST_CODE: 0,
    ECHO_REPLY_TYPE: 0,
    ECHO_REPLY_CODE: 0,
    HEADER_SIZE: 8,
};

const ICMPV6_CONST: ProtoTypeConsts = ProtoTypeConsts {
    isV4: false,
    ver: "V6",
    ECHO_REQUEST_TYPE: 128,
    ECHO_REQUEST_CODE: 0,
    ECHO_REPLY_TYPE: 129,
    ECHO_REPLY_CODE: 0,
    HEADER_SIZE: 8,
};

fn write_checksum(buffer: &mut [u8]) {
    // we reset these for buffer reuse
    buffer[2] = 0;
    buffer[3] = 0;

    let mut sum = 0u32;
    for word in buffer.chunks(2) {
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

    buffer[2] = (sum >> 8) as u8;
    buffer[3] = (sum & 0xff) as u8;
}

#[derive(Debug)]
struct IcmpEchoReply {
    pub type_: u8,
    pub code: u8,
    pub ident: u16,
    pub seq: u16,
}

impl IcmpEchoReply {
    pub fn decode(buf: &[u8], proto: &ProtoTypeConsts) -> Result<Option<Self>, anyhow::Error> {
        let mut header_size = 0usize;
        if proto.isV4 {
            let byte0 = buf[0];
            let version = (byte0 & 0xf0) >> 4;
            header_size = 4 * ((byte0 & 0x0f) as usize);
            let ttl = buf[8];
        }

        let icmp_data = &buf[header_size..];

        let type_ = icmp_data[0];
        let code = icmp_data[1];
        if type_ != proto.ECHO_REPLY_TYPE && code != proto.ECHO_REPLY_CODE {
            return Ok(None);
        }

        let ident = (u16::from(icmp_data[4]) << 8) + u16::from(icmp_data[5]);
        let seq = (u16::from(icmp_data[6]) << 8) + u16::from(icmp_data[7]);
        Ok(Some(IcmpEchoReply {
            type_,
            code,
            ident,
            seq,
        }))
    }
}

