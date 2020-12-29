#![allow(unused_imports, unused_variables, unused_mut, unused_parens)]

use std::{env, fmt};
use std::convert::{Infallible, TryInto};
use std::env::Args;
use std::fmt::{Display, Formatter};
use std::net::{IpAddr, ToSocketAddrs};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering, AtomicBool};
use std::time::{Duration, Instant, SystemTime};

use anyhow::{bail, anyhow, Context, Result};
use anyhow::private::kind::{AdhocKind, TraitKind};
use humantime::{format_rfc3339_millis, parse_duration};
use socket2::SockAddr;
use structopt::StructOpt;

use cli::*;
use stats::{Stats, stats_thread};
use util::*;
use crate::stop::Stop;

mod ipv4;
mod icmp;
mod ping;
mod stats;
mod util;
mod cli;
mod stop;

fn main() {
    match run() {
        Ok(_) => {}
        Err(e) => { eprintln!("error: {:#?}", e) }
    }
}


fn run() -> Result<()> {
    let cfg: Config = Config::from_args();
    if cfg.verbose > 1 {
        println!("options: \n{:#?}", &cfg);
    }
    let mut stop = Stop::new();

    println!("[{}]  starting....", format_rfc3339_millis(SystemTime::now()));

    let mut threads = vec![];
    let mut tracks = vec![];
    for (no, ip) in cfg.ips.iter().enumerate() {
        let mut stats = Arc::new(Stats::new());
        tracks.push(stats.clone());
        let (ip, verbose, interval, timeout, stats) = (ip.clone(), cfg.verbose, cfg.interval, cfg.timeout, stats.clone());
        threads.push(std::thread::Builder::new()
            .name(format!("ping{}", no))
            .spawn(move || ping_thread(ip, verbose, interval, timeout, stats))?);
    }
    if cfg.verbose > 1 {
        println!("all ping threads started");
    }
    let mut ip_list = vec![];
    for ip in &cfg.ips {
        let a = ip.clone();
        ip_list.push(a);
    }

    if cfg.stat_interval.as_millis() > 0 {
        let interval = cfg.stat_interval;
        let mut stop = stop.clone();
        if cfg.verbose > 1 {
            println!("starting stats thread");
        }
        let tracker_h = std::thread::Builder::new()
            .name(String::from("stats"))
            .spawn(move || stats_thread(stop, interval, ip_list, tracks))?;
    } else {
        println!("no stats tracking started");
    }

    for h in threads {
        h.join();
    }
    Ok(())
}

fn ping_thread(hostinfo: HostInfo, verbose: usize, interval: Duration, timeout: Duration, stats: Arc<Stats>) {
    const PING_IDENT: u16 = 2112;
    let mut seq_cnt = 0u16;
    println!("starting thread for {}", &hostinfo);
    let mut buff = String::with_capacity(128);
    loop {
        let now = Instant::now();
        let res = ping::ping(hostinfo.ip, Some(timeout),
                             Some(255), Some(PING_IDENT), Some(seq_cnt), None, verbose);
        let dur = now.elapsed();
        let st = SystemTime::now();
        let iii = 100;
        match res {
            Ok((ret_size, ret_sockaddr, ret_ident, ret_seqcnt, ret_hops)) => {
                let micros = (dur.as_nanos() / 1000) as u64;
                stats.update_micros_working(micros);

                if ret_ident != PING_IDENT || seq_cnt != ret_seqcnt {
                    buff.clear();
                    use std::fmt::Write;
                    writeln!(&mut buff, "[{}] response differences for {}", format_rfc3339_millis(st), &hostinfo);
                    if hostinfo.ip != ret_sockaddr.as_std().unwrap().ip() {
                        let ret_ip_disp = SockAddrWrap { wrap: &ret_sockaddr };
                        writeln!(&mut buff, "\tIpAddr sent: {}  return: {}", hostinfo.ip, ret_ip_disp);
                    }
                    if ret_ident != PING_IDENT {
                        writeln!(&mut buff, "\tdent: sent: {}  return: {}", PING_IDENT, ret_ident);
                    }
                    if seq_cnt != ret_seqcnt {
                        writeln!(&mut buff, "\tseqcnt: sent: {}  return: {}", seq_cnt, ret_seqcnt);
                    }
                    println!("{}", &buff);
                }

                if verbose > 0 {
                    if hostinfo.ip == ret_sockaddr.as_std().unwrap().ip() {
                        println!("[{}]  success for {} in {} ttl={} seq={}",
                                 format_rfc3339_millis(st), &hostinfo, util::format_duration_mine(dur),
                                 ret_hops, seq_cnt);
                    } else {
                        let ret_ip_disp = SockAddrWrap { wrap: &ret_sockaddr };
                        println!("[{}]  success for {} in {} diff ret addr: {}",
                                 format_rfc3339_millis(st),
                                 &hostinfo, util::format_duration_mine(dur),
                                 ret_ip_disp
                        );
                    }
                    if verbose > 1 {
                        let ret_ip_disp = SockAddrWrap { wrap: &ret_sockaddr };
                        println!("\tret addr: {} ret size: {} ident: {} seq_cnt {}  ", ret_ip_disp, ret_size, ret_ident, ret_seqcnt);
                    }
                }
            }
            Err(e) => {
                stats.update_fail();
                if let Some(ee) = e.downcast_ref::<std::io::Error>() {
                    match ee.kind() {
                        std::io::ErrorKind::TimedOut => println!("[{}]  timeout for ip {} seq_cnt: {} after {}", format_rfc3339_millis(st), &hostinfo, seq_cnt, util::format_duration_mine(dur)),
                        _ => {
                            print!("[{}]  io error for {}: {}", format_rfc3339_millis(st), &hostinfo, e);
                            e.chain().skip(1).for_each(|cause| print!(" // {}", cause));
                            println!();
                        }
                        //_ => println!("[{}]  io error for ip {}, error: {:#?}", format_rfc3339_millis(st), &ip, e),
                    }
                } else {
                    println!("[{}]  general error for {} of {:?}", format_rfc3339_millis(st), &hostinfo, e);
                }
            }
        }
        std::thread::sleep(interval);
        seq_cnt = seq_cnt.wrapping_add(1);
    }
}

