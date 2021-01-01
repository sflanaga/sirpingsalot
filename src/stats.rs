use humantime::format_rfc3339_millis;
use std::sync::atomic::{AtomicU64, Ordering, AtomicBool};
use std::time::{Duration, SystemTime};
use std::net::IpAddr;
use std::sync::Arc;

use crate::util::sleep_until_next_interval_on;
use std::fmt;
use crate::cli::HostInfo;
use crate::stop::Stop;

pub struct Stats {
    reply: AtomicU64,
    non_reply: AtomicU64,
    timeout: AtomicU64,
    time_sum_us: AtomicU64,
    time_min_us: AtomicU64,
    time_max_us: AtomicU64,
}

impl Stats {
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
}

pub fn stats_thread(mut running: Stop, interval: Duration, ip_list: Vec<HostInfo>, tracks: Vec<Arc<Stats>>) {
    use tabular::{Table, Row};
    loop {
        let stop =  sleep_until_next_interval_on(&mut running, interval);
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
        //table.add_heading("host worked failed avg min max");
        // table.with_heading()
        //     .add_heading("host")
        //     .add_heading("worked")
        //     .add_heading("failed")
        //     .add_heading("avg")
        //     .add_heading("min")
        //     .add_heading("max");
        let st = SystemTime::now();
        for i in 0..ip_list.len() {
            let ip = ip_list.get(i).unwrap();
            let reply = tracks.get(i).unwrap().reply.swap(0, Ordering::Relaxed);
            let non_reply = tracks.get(i).unwrap().non_reply.swap(0, Ordering::Relaxed);
            let timeout = tracks.get(i).unwrap().timeout.swap(0, Ordering::Relaxed);
            let sum = tracks.get(i).unwrap().time_sum_us.swap(0, Ordering::Relaxed);
            let min = tracks.get(i).unwrap().time_min_us.swap(std::u64::MAX, Ordering::Relaxed);
            let max = tracks.get(i).unwrap().time_max_us.swap(0, Ordering::Relaxed);
            if reply > 0 || non_reply > 0 {
                let avg_ms = (sum as f64 / (non_reply + reply) as f64) / 1000f64;
                let min_ms = min as f64 / 1000f64;
                let max_ms = max as f64 / 1000f64;
                table.add_row(Row::new()
                    .with_cell(&ip)
                    .with_cell(reply)
                    .with_cell(non_reply)
                    .with_cell(timeout)
                    .with_cell(format!("{:.3}", avg_ms))
                    .with_cell(format!("{:.3}", min_ms))
                    .with_cell(format!("{:.3}", max_ms)));
                // println!("[{}] {}  worked: {}  failed: {}  times, avg: {:.3}ms  min: {:.3}ms  max: {:.3}ms",
                //          format_rfc3339_millis(st), &ip, worked, failed,
                //          avg_ms, min_ms, max_ms);
            } else {
                table.add_row(Row::new()
                    .with_cell(&ip)
                    .with_cell(reply)
                    .with_cell(non_reply)
                    .with_cell(timeout)
                    .with_cell("NA")
                    .with_cell("NA")
                    .with_cell("NA")
                );
            }
        }
        if stop {
            println!("[{}]  FINAL/EARLY dump of STATS:", format_rfc3339_millis(st));
            print!("{}", table);
            std::process::exit(0);
        } else {
            println!("[{}]  STATS:", format_rfc3339_millis(st));
            print!("{}", table);
        }
    }
}


