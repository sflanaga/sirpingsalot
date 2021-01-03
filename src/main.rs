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
use log::{debug, error, info, trace, warn};

use cli::*;
use stats::{Stats, stats_thread};
use util::*;
use crate::stop::Stop;
use crate::ping::*;

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
    init_log(cfg.log_level);
    debug!("options: \n{:#?}", &cfg);
    let mut stop = Stop::new();

    info!("[{}]  starting....", format_rfc3339_millis(SystemTime::now()));

    let mut threads = vec![];
    let mut tracks = vec![];
    let mut seq_cnt_start = 4000;
    for (no, ip) in cfg.ips.iter().enumerate() {
        seq_cnt_start += 111u16;

        let mut stats = Arc::new(Stats::new());
        tracks.push(stats.clone());
        let (ip, no, raw_write_odd, interval, timeout, stats) = (ip.clone(), no, cfg.raw_write_odd, cfg.interval, cfg.timeout, stats.clone());
        threads.push(std::thread::Builder::new()
            .name(format!("ping{}", no))
            .spawn(move || ping_thread2(ip, no, raw_write_odd, interval, timeout, stats))?);
    }
    debug!("all ping threads started");
    let mut ip_list = vec![];
    for ip in &cfg.ips {
        let a = ip.clone();
        ip_list.push(a);
    }

    if cfg.stat_interval.as_millis() > 0 {
        let interval = cfg.stat_interval;
        let mut stop = stop.clone();
        debug!("starting stats thread");
        let tracker_h = std::thread::Builder::new()
            .name(String::from("stats"))
            .spawn(move || stats_thread(stop, interval, ip_list, tracks))?;
    } else {
        info!("no stats tracking started");
        while !stop.sleep(Duration::from_secs(60)) {
            ;
        }
        std::process::exit(0);
    }

    for h in threads {
        h.join();
    }
    Ok(())
}

fn ping_thread2(hostinfo: HostInfo, no: usize, raw_write_odd: bool, interval: Duration, timeout: Duration, stats: Arc<Stats>) {
    const PING_IDENT: u16 = 2112    ;
    let mut seq_cnt = (100 + no *100) as u16;
    debug!("starting thread for {}", &hostinfo);

    let mut pinger = match Pinger::new(hostinfo.ip, timeout) {
        Err(e) => {
            error!("failed to setup ping for {} with error {:?}", hostinfo.ip, e);
            std::process::exit(10);
        },
        Ok(v) => v,
    };

    let mut buff= String::with_capacity(128);
    std::thread::sleep(Duration::from_millis((no * 100) as u64));
    loop {
        let now = Instant::now();
        let res = pinger.ping1(PING_IDENT, seq_cnt, 255);
        let dur = now.elapsed();
        match res {
            Ok((ret_size, ret_sockaddr)) => {
                let ret_dec = pinger.decode();
                match ret_dec {
                    Ok((ret_type, ret_code, ret_ident, ret_seq)) => {
                        let micros = (dur.as_nanos() / 1000) as u64;
                        if ret_type != 0 && ret_type != 129 {
                            stats.update_micros_non_reply(micros);
                        } else {
                            stats.update_micros_working(micros);
                        }
                        trace!("{} RAW return: {:02X?}", &hostinfo, pinger.get_recv_buffer(ret_size));

                        // if raw_write_odd {
                            if ret_ident != PING_IDENT || seq_cnt != ret_seq {
                                buff.clear();
                                use std::fmt::Write;
                                writeln!(&mut buff, "response differences for {} time={}", &hostinfo, util::format_duration_mine(dur));
                                if hostinfo.ip != ret_sockaddr.as_std().unwrap().ip() {
                                    let ret_ip_disp = SockAddrWrap { wrap: &ret_sockaddr };
                                    writeln!(&mut buff, "\tIpAddr sent: {}  return: {}", hostinfo.ip, ret_ip_disp);
                                }
                                if ret_ident != PING_IDENT {
                                    writeln!(&mut buff, "\tident: sent: {}  return: {}", PING_IDENT, ret_ident);
                                }
                                if seq_cnt != ret_seq {
                                    writeln!(&mut buff, "\tseqcnt: sent: {}  return: {}", seq_cnt, ret_seq);
                                }
                                info!("{}", &buff);
                            }
                        // }
                        // if verbose > 0 {
                        //     if hostinfo.ip == ret_sockaddr.as_std().unwrap().ip() {
                        //         println!("[{}]  success for {} in {} ttl={} seq={}",
                        //                  format_rfc3339_millis(st), &hostinfo, util::format_duration_mine(dur),
                        //                  ret_hops, seq_cnt);
                        //     } else {
                        //         let ret_ip_disp = SockAddrWrap { wrap: &ret_sockaddr };
                        //         println!("[{}]  success for {} in {} diff ret addr: {}",
                        //                  format_rfc3339_millis(st),
                        //                  &hostinfo, util::format_duration_mine(dur),
                        //                  ret_ip_disp
                        //         );
                        //     }
                        //     if verbose > 1 {
                        //         let ret_ip_disp = SockAddrWrap { wrap: &ret_sockaddr };
                        //         println!("\tret addr: {} ret size: {} ident: {} seq_cnt {}  raw: {}", ret_ip_disp, ret_size, ret_ident, ret_seqcnt, &raw_pack_info);
                        //         if verbose > 2 {
                        //             println!("\tRAW: {}", &raw_pack_info);
                        //         }
                        //     }
                        // }
                        info!("success for {} in {:?}", hostinfo, dur);
                    },
                    Err(e) => println!("error decoding return packet from {}, {}", hostinfo, e),
                }
            },
            Err(e)=> {
                stats.update_fail();

                warn!("error for {} after {:?}, {}", hostinfo, dur, e);
                e.chain().skip(1).for_each(|cause| warn!("\t cause: {}", cause));

            }
        }
        std::thread::sleep(interval);
        seq_cnt = seq_cnt.wrapping_add(1);

    }

}


fn ping_thread(hostinfo: HostInfo, raw_write_odd: bool, verbose: usize, interval: Duration, timeout: Duration, stats: Arc<Stats>) {
    const PING_IDENT: u16 = 2112;
    let mut seq_cnt = 0u16;
    println!("starting thread for {}", &hostinfo);
    let mut buff = String::with_capacity(128);
    let mut raw_pack_info = String::with_capacity(128);
    let record_raw_bits = raw_write_odd || verbose > 2;
    loop {
        let now = Instant::now();
        let res = ping::ping(hostinfo.ip, Some(timeout),
                             Some(255), Some(PING_IDENT), Some(seq_cnt), None, verbose, &mut raw_pack_info, record_raw_bits);
        let dur = now.elapsed();
        let st = SystemTime::now();
        let iii = 100;
        match res {
            Ok((ret_size, ret_sockaddr, ret_type, ret_code, ret_ident, ret_seqcnt, ret_hops)) => {
                let micros = (dur.as_nanos() / 1000) as u64;
                if ret_type != 0 && ret_type != 129 {
                    stats.update_micros_non_reply(micros);
                } else {
                    stats.update_micros_working(micros);
                }

                if raw_write_odd {
                    if ret_ident != PING_IDENT || seq_cnt != ret_seqcnt {
                        buff.clear();
                        use std::fmt::Write;
                        writeln!(&mut buff, "[{}] response differences for {} time={}", format_rfc3339_millis(st), &hostinfo, util::format_duration_mine(dur));
                        if hostinfo.ip != ret_sockaddr.as_std().unwrap().ip() {
                            let ret_ip_disp = SockAddrWrap { wrap: &ret_sockaddr };
                            writeln!(&mut buff, "\tIpAddr sent: {}  return: {}", hostinfo.ip, ret_ip_disp);
                        }
                        if ret_ident != PING_IDENT {
                            writeln!(&mut buff, "\tident: sent: {}  return: {}", PING_IDENT, ret_ident);
                        }
                        if seq_cnt != ret_seqcnt {
                            writeln!(&mut buff, "\tseqcnt: sent: {}  return: {}", seq_cnt, ret_seqcnt);
                        }
                        println!("{}\tRAW:{}", &buff, &raw_pack_info);
                    }
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
                        println!("\tret addr: {} ret size: {} ident: {} seq_cnt {}  raw: {}", ret_ip_disp, ret_size, ret_ident, ret_seqcnt, &raw_pack_info);
                        if verbose > 2 {
                            println!("\tRAW: {}", &raw_pack_info);
                        }
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

