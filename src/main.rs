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

    info!("[{}]  starting....", format_rfc3339_millis(SystemTime::now()));

    let mut threads = vec![];
    for (no, ip) in cfg.ips.iter().enumerate() {
        let ip: HostInfo = ip.clone();
        let (interval, timeout) = (cfg.interval, cfg.timeout);
        threads.push(std::thread::Builder::new()
            .name(format!("ping{}", no))
            .spawn(move || ping_thread(ip, no, interval, timeout))?);
    }
    debug!("all ping threads started");

    if cfg.stat_interval.as_millis() > 0 {
        let interval = cfg.stat_interval;
        let stop = stop.clone();
        let tracker = Tracks::new(&cfg)?;
        debug!("starting stats thread");
        let _tracker_h = std::thread::Builder::new()
            .name(String::from("stats"))
            .spawn(move || stats::stats_thread(tracker, stop, interval))?;
    } else {
        info!("no stats tracking started");
        while !stop.sleep(Duration::from_secs(60)) {
        }
        std::process::exit(0);
    }

    for h in threads {
        let _ = h.join();
    }
    Ok(())
}

fn ping_thread(hostinfo: HostInfo, no: usize, interval: Duration, timeout: Duration) {
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
        let res = pinger.ping1(ping_ident, seq_cnt, 255);
        let dur = now.elapsed();
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
                            info!("success for {} in {:?}", hostinfo, dur);
                        }
                    },
                    Err(e) => error!("error decoding return packet from {}, {}", hostinfo, e),
                }
            },
            Err(e)=> {
                warn!("error for {} after {:?}, {}", hostinfo, dur, e);
                e.chain().skip(1).for_each(|cause| warn!("\t cause: {}", cause));
            }
        }
        std::thread::sleep(interval);
        seq_cnt = seq_cnt.wrapping_add(1);

    }

}

