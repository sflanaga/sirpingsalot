#![allow(unused_imports, unused_variables, unused_mut, unused_parens)]

use std::{env, fmt};
use std::convert::{Infallible, TryInto};
use std::env::Args;
use std::fmt::{Display, Formatter};
use std::net::{IpAddr, ToSocketAddrs};
use std::result::Result;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

use anyhow::{anyhow, Context};
use anyhow::private::kind::{AdhocKind, TraitKind};
use humantime::{format_rfc3339_millis, parse_duration};
use socket2::SockAddr;
use structopt::StructOpt;

use cli::*;
use stats::{Stats, stats_thread};
use util::*;

mod ipv4;
mod icmp;
mod ping;
mod stats;
mod util;
mod cli;

type ResultS<T> = std::result::Result<T, anyhow::Error>;

fn main() {
    match run() {
        Ok(_) => {}
        Err(e) => { eprintln!("error: {:#?}", e) }
    }
}

fn run() -> ResultS<()> {
    let cfg: Config = Config::from_args();
    if cfg.verbose > 1 {
        println!("options: \n{:#?}", &cfg);
    }

    println!("[{}]  starting....", format_rfc3339_millis(SystemTime::now()));

    let mut threads = vec![];
    let mut tracks = vec![];
    for ip in &cfg.ips {
        let mut stats = Arc::new(Stats::new());
        tracks.push(stats.clone());
        let (ip, verbose, interval, timeout, stats) = (ip.clone(), cfg.verbose, cfg.interval, cfg.timeout, stats.clone());
        threads.push(std::thread::spawn(move || ping_thread(ip, verbose, interval, timeout, stats)));
    }
    if cfg.verbose > 1  {
        println!("all ping threads started");
    }
    let mut ip_list = vec![];
    for ip in &cfg.ips {
        let a = ip.clone();
        ip_list.push(a);
    }

    if cfg.stat_interval.as_millis() > 0 {
        let interval = cfg.stat_interval;
        if cfg.verbose > 1 {
            println!("starting stats thread");
        }
        let tracker_h = std::thread::spawn(move || stats_thread(interval, ip_list, tracks));
    } else {
        println!("no stats tracking started");
    }

    for h in threads {
        h.join();
    }
    Ok(())
}

fn ping_thread(ip: IpAddr, verbose: usize, interval: Duration, timeout: Duration, stats: Arc<Stats>) {
    let mut seq_cnt = 0u16;
    println!("starting thread for {}", &ip);
    loop {
        let now = Instant::now();
        let res = ping::ping(ip, Some(timeout),
                             Some(255), Some(2112), Some(seq_cnt), None, verbose);
        let dur = now.elapsed();
        let st = SystemTime::now();
        seq_cnt = seq_cnt.wrapping_add(1);
        match res {
            Ok((ret_size, ret_sockaddr, ret_ident, ret_seqcnt)) => {
                let micros = (dur.as_nanos() / 1000) as u64;
                stats.update_micros_working(micros);
                if verbose > 0 {
                    println!("[{}]  success for ip: {} in {}", format_rfc3339_millis(st), &ip, util::format_duration_mine(dur));
                    if verbose > 1 {
                        let ip_disp = SockAddrWrap { wrap: &ret_sockaddr };
                        println!("\tret addr: {} ret size: {} ident: {} seq_cnt {}  ", ip_disp, ret_size, ret_ident, ret_seqcnt);
                    }
                }
            }
            Err(e) => {
                stats.update_fail();
                if let Some(e) = e.downcast_ref::<std::io::Error>() {
                    match e.kind() {
                        std::io::ErrorKind::TimedOut => println!("[{}]  timeout for ip {} seq_cnt: {} after {}", format_rfc3339_millis(st), &ip, seq_cnt, util::format_duration_mine(dur)),
                        _ => println!("[{}]  io error for ip {}, error: {}", format_rfc3339_millis(st), &ip, e),
                    }
                } else {
                    println!("[{}]  general error for ip {} of {:?}", format_rfc3339_millis(st), &ip, e);
                }
            }
        }
        std::thread::sleep(interval);
    }
}

