use std::time::{Duration, Instant, SystemTime};

use anyhow::Result;
use humantime::format_rfc3339_millis;
use rand::Rng;
use clap::Parser;
use log::{debug, error, info, trace, warn};

use cli::*;
use stats::Tracks;
use util::*;
use crate::stop::Stop;
use crate::ping::*;

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
    let cfg: Config = Config::parse();
    init_log(cfg.log_level);
    debug!("options: \n{:#?}", &cfg);
    let stop = Stop::new();

    // Compute or validate stat_interval.
    // 0 (the default) means auto-select the smallest value from the sequence
    // [60s, 300s, 900s, 1h, 4h, 24h] that exceeds the ping interval.
    // An explicitly provided value must be greater than the ping interval.
    let stat_interval = if cfg.stat_interval.as_millis() == 0 {
        const DEFAULTS: &[u64] = &[60, 300, 900, 3_600, 14_400, 86_400];
        DEFAULTS.iter()
            .map(|&s| Duration::from_secs(s))
            .find(|&d| d > cfg.interval)
            .unwrap_or(Duration::from_secs(86_400))
    } else {
        if cfg.stat_interval <= cfg.interval {
            return Err(anyhow::anyhow!(
                "stat interval (-s {:?}) must be greater than ping interval (-i {:?})",
                cfg.stat_interval, cfg.interval
            ));
        }
        cfg.stat_interval
    };

    info!("[{}]  starting.... (ping interval: {:?}, stat interval: {:?})",
        format_rfc3339_millis(SystemTime::now()), cfg.interval, stat_interval);

    let tracker = Tracks::new(&cfg)?;

    let mut threads = vec![];
    for (no, ip) in cfg.ips.iter().enumerate() {
        let ip: HostInfo = ip.clone();
        let (interval, timeout) = (cfg.interval, cfg.timeout);
        let tracker = tracker.clone();
        threads.push(std::thread::Builder::new()
            .name(format!("ping{}", no))
            .spawn(move || ping_thread(ip, no, interval, timeout, tracker))?);
    }
    debug!("all ping threads started");

    {
        let stop = stop.clone();
        debug!("starting stats thread with interval {:?}", stat_interval);
        let _tracker_h = std::thread::Builder::new()
            .name(String::from("stats"))
            .spawn(move || stats::stats_thread(tracker, stop, stat_interval))?;
    }

    for h in threads {
        let _ = h.join();
    }
    Ok(())
}

fn ping_thread(hostinfo: HostInfo, no: usize, interval: Duration, timeout: Duration, mut tracker: Tracks) {
    let ping_ident: u16 = rand::rng().random();
    let mut seq_cnt = (100 + no * 100) as u16;
    debug!("starting thread for {} ident={}", &hostinfo, ping_ident);

    let mut pinger = match Pinger::new(hostinfo.ip, timeout, hostinfo.to_string()) {
        Err(e) => {
            error!("failed to setup ping for {} with error {:?}", hostinfo.ip, e);
            std::process::exit(10);
        },
        Ok(v) => v,
    };

    let mut buff = String::with_capacity(128);
    std::thread::sleep(Duration::from_millis((no * 100) as u64));
    loop {
        let now = Instant::now();
        tracker.update_for_send(hostinfo.ip, now, ping_ident, seq_cnt);
        let res = pinger.ping1(ping_ident, seq_cnt, 255);
        let recv_instant = Instant::now();
        let dur = recv_instant - now;
        match res {
            Ok((ret_size, ret_sockaddr)) => {
                match pinger.decode() {
                    Ok((_ret_type, _ret_code, ret_ident, ret_seq)) => {
                        trace!("{} RAW return: {:02X?}", &hostinfo, pinger.get_recv_buffer(ret_size));

                        if ret_ident != ping_ident || seq_cnt != ret_seq {
                            buff.clear();
                            use std::fmt::Write;
                            let _ = writeln!(&mut buff, "response differences for {} time={}", &hostinfo, util::format_duration_mine(dur));
                            if hostinfo.ip != ret_sockaddr.as_socket().unwrap().ip() {
                                let ret_ip_disp = SockAddrWrap { wrap: &ret_sockaddr };
                                let _ = writeln!(&mut buff, "\tIpAddr sent: {}  return: {}", hostinfo.ip, ret_ip_disp);
                            }
                            if ret_ident != ping_ident {
                                let _ = writeln!(&mut buff, "\tident: sent: {}  return: {}", ping_ident, ret_ident);
                            }
                            if seq_cnt != ret_seq {
                                let _ = writeln!(&mut buff, "\tseqcnt: sent: {}  return: {}", seq_cnt, ret_seq);
                            }
                            warn!("{}", &buff);
                        } else {
                            tracker.update_for_recv(hostinfo.ip, recv_instant, ret_ident, ret_seq);
                            info!("success for {} in {:?}", hostinfo, dur);
                        }
                    },
                    Err(e) => error!("error decoding return packet from {}, {}", hostinfo, e),
                }
            },
            Err(e)=> {
                let causes: Vec<String> = e.chain().skip(1).map(|c| c.to_string()).collect();
                if causes.is_empty() {
                    warn!("error for {} after {:?}, {}", hostinfo, dur, e);
                } else {
                    warn!("error for {} after {:?}, {} ({})", hostinfo, dur, e, causes.join("; "));
                }
            }
        }
        std::thread::sleep(interval);
        seq_cnt = seq_cnt.wrapping_add(1);

    }

}

