#![allow(unused_imports, unused_variables, unused_mut, unused_parens)]
use humantime::format_rfc3339_millis;
use std::sync::atomic::{AtomicU64, Ordering, AtomicBool};
use std::time::{Duration, SystemTime, Instant};
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use log::{debug, error, info, trace, warn};
use anyhow::{Context, anyhow};


use crate::util::sleep_until_next_interval_on;
use std::fmt;
use crate::cli::{HostInfo, Config};
use crate::stop::Stop;
use std::collections::HashMap;
use tabular::{Table, Row};
use socket2::SockAddr;

pub struct Stats {
    reply: AtomicU64,
    non_reply: AtomicU64,
    timeout: AtomicU64,
    time_sum_us: AtomicU64,
    time_min_us: AtomicU64,
    time_max_us: AtomicU64,
}

pub struct StatsSnapShot {
    reply: u64,
    non_reply: u64,
    timeout: u64,
    time_sum_us: u64,
    time_min_us: u64,
    time_max_us: u64,
}

impl Stats {
    pub fn new() -> Stats {
        Stats {
            reply: AtomicU64::new(0),
            timeout: AtomicU64::new(0),
            non_reply: AtomicU64::new(0),
            time_sum_us: AtomicU64::new(0),
            time_min_us: AtomicU64::new(std::u64::MAX),
            time_max_us: AtomicU64::new(0),
        }
    }

    pub fn zero_extract(&mut self) -> StatsSnapShot {
        StatsSnapShot {
            reply:self.reply.swap(0, Ordering::Relaxed),
            non_reply:self.non_reply.swap(0, Ordering::Relaxed),
            timeout:self.timeout.swap(0, Ordering::Relaxed),
            time_sum_us:self.time_sum_us.swap(0, Ordering::Relaxed),
            time_min_us:self.time_min_us.swap(std::u64::MAX, Ordering::Relaxed),
            time_max_us:self.time_max_us.swap(0, Ordering::Relaxed),
        }
    }

    pub fn update_micros_working(&self, micros: u64) {
        self.reply.fetch_add(1, Ordering::Relaxed);
        self.time_sum_us.fetch_add(micros, Ordering::Relaxed);
        self.time_min_us.fetch_min(micros, Ordering::Relaxed);
        self.time_max_us.fetch_max(micros, Ordering::Relaxed);
    }

    pub fn update_micros_non_reply(&self, micros: u64) {
        self.non_reply.fetch_add(1, Ordering::Relaxed);
        self.time_sum_us.fetch_add(micros, Ordering::Relaxed);
        self.time_min_us.fetch_min(micros, Ordering::Relaxed);
        self.time_max_us.fetch_max(micros, Ordering::Relaxed);
    }

    pub fn update_fail(&self) {
        self.timeout.fetch_add(1, Ordering::Relaxed);
    }
}

impl Stats {
}


pub fn stats_thread(mut tracker: Tracks, mut running: Stop, interval:Duration) {
    use tabular::{Table, Row};
    loop {
        let stop =  sleep_until_next_interval_on(&mut running, interval);

        let table = tracker.create_stats_table();
        if stop {
            error!("FINAL/EARLY dump of STATS:\n{}", table);
            std::process::exit(0);
        } else {
            error!("STATS:\n{}", table);
        }
    }
}

struct TrackPerHost {
    host: HostInfo,
    last_time: Option<Instant>,
    ident: u16,
    last_seq: Option<u16>,
    mark: bool,
    stats: Stats,
}

struct TracksInner {
    map: HashMap<std::net::IpAddr, TrackPerHost>,
}

pub struct Tracks {
    inner: Arc<Mutex<TracksInner>>,
}

impl Clone for Tracks {
    fn clone(&self) -> Self {
        Tracks {
            inner: self.inner.clone()
        }
    }
}

pub struct UpdateSendIteration {
    pub ident: u16,
    pub ip: IpAddr,
    pub sa: SockAddr,
    pub now: Instant,
}

impl Tracks {
    pub fn new(cfg: &Config) -> Result<Self, anyhow::Error>
    {
        let mut ident = cfg.ident_base;
        let mut map = HashMap::new();
        for h in &cfg.ips {
            if map.contains_key(&h.ip) {
                return Err(anyhow!("duplicate ip for {}", h));
            } else {
                map.insert(h.ip.clone(), TrackPerHost {
                    host: h.clone(),
                    last_time: None,
                    last_seq: None,
                    ident,
                    mark: false,
                    stats: Stats::new(),
                });
            }
            ident = ident.wrapping_add(1);
        }
        Ok(Tracks {
            inner: Arc::new(Mutex::new(TracksInner { map }))
        })
    }

    pub fn update_for_recv(&mut self, ip: IpAddr, now: Instant, ident: u16, seq: u16) -> bool {
        let mut lock = self.inner.lock().unwrap();
        if let Some(per_host) = lock.map.get_mut(&ip) {
            if per_host.ident == ident {
                info!("ident difference for {} expected: {} got {}", per_host.host,
                      per_host.ident, ident);
            }
            if per_host.last_seq.expect("should get a seqence") != seq {
                info!("seq out of order for {}: sent: {} but just got {}", per_host.host,
                      per_host.last_seq.unwrap(), seq);
            }
            let dur = now - per_host.last_time.expect("was supposed to have sometime");
            per_host.stats.update_micros_working(dur.as_micros() as u64);
            debug!("success for {} time: {:?}", per_host.host, dur);
            per_host.mark = true;
            true
        } else {
            false
        }
    }
    pub fn update_for_send(&mut self, ip: IpAddr, now: Instant, ident: u16, seq: u16) -> bool {
        let mut lock = self.inner.lock().unwrap();
        let mut per_host = lock.map.get_mut(&ip).expect("hey - this ip should be there but is not");
        let last_mark = per_host.mark;
        if !per_host.mark && per_host.last_seq.is_some() {
            info!("timeout for {} missed seq {}", per_host.host, per_host.last_seq.unwrap());
            per_host.stats.update_fail();
        }
        per_host.last_seq = Some(seq);
        per_host.last_time = Some(now);
        per_host.mark = false;
        last_mark
    }

    pub fn update_for_send_bulk(&mut self, v: &Vec<UpdateSendIteration>, seq: u16) {
        let mut lock = self.inner.lock().unwrap();
        for i in v.iter() {
            let mut per_host = lock.map.get_mut(&i.ip).expect("hey - this ip should be there but is not");
            if !per_host.mark && per_host.last_seq.is_some() {
                info!("timeout for {} missed seq {}", per_host.host, per_host.last_seq.unwrap());
                per_host.stats.update_fail();
            }
            per_host.last_seq = Some(seq);
            per_host.last_time = Some(i.now);
            per_host.mark = false;
        }
    }


    pub fn create_stats_table(&mut self) -> Table {
        let mut table = Table::new("\t{:<} {:>} {:>} {:>} {:>} {:>} {:>}");
        table.add_row(Row::new()
            .with_cell("host")
            .with_cell("reply")
            .with_cell("nonreply")
            .with_cell("timeout")
            .with_cell("avg(ms)")
            .with_cell("min(ms)")
            .with_cell("max(ms)")
        );
        let st = SystemTime::now();
        {
            let stat_vec = self.inner.lock().unwrap()
                .map.iter_mut()
                .map(|(ip, v)| (ip.clone(),v.host.clone(),v.stats.zero_extract())).collect::<Vec<_>>();
            for (ip, h, stat) in stat_vec.iter() {
                if stat.reply > 0 || stat.non_reply > 0 {
                    let avg_ms = (stat.time_sum_us as f64 / (stat.non_reply + stat.reply) as f64) / 1000f64;
                    let min_ms = stat.time_min_us as f64 / 1000f64;
                    let max_ms = stat.time_max_us as f64 / 1000f64;
                    table.add_row(Row::new()
                        .with_cell(&h)
                        .with_cell(stat.reply)
                        .with_cell(stat.non_reply)
                        .with_cell(stat.timeout)
                        .with_cell(format!("{:.3}", avg_ms))
                        .with_cell(format!("{:.3}", min_ms))
                        .with_cell(format!("{:.3}", max_ms)));
                } else {
                    table.add_row(Row::new()
                        .with_cell(&h)
                        .with_cell(stat.reply)
                        .with_cell(stat.non_reply)
                        .with_cell(stat.timeout)
                        .with_cell("NA")
                        .with_cell("NA")
                        .with_cell("NA")
                    );
                }
            }
        }
        table
    }
}

