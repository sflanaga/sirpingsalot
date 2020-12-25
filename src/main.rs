#![allow(unused_imports)]

mod ipv4;
mod icmp;
mod ping;

use std::{env, fmt};
use std::net::{IpAddr, ToSocketAddrs};
use std::time::{Duration, SystemTime, Instant};
use humantime::{parse_duration, format_rfc3339_millis};
use anyhow::{anyhow, Context};
use std::env::Args;

type ResultS<T> = std::result::Result<T, anyhow::Error>;

use std::result::Result;
use anyhow::private::kind::{AdhocKind, TraitKind};
use std::convert::{TryInto, Infallible};
use structopt::StructOpt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Clone)]
pub struct FDur(Duration);

pub fn format_duration_mine(val: Duration) -> FDur {
    FDur(val)
}

impl fmt::Display for FDur {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let secs = self.0.as_secs();
        let nanos = self.0.subsec_nanos();

        if secs == 0 && nanos == 0 {
            f.write_str("0s")?;
            return Ok(());
        }

        if secs == 0 {
            if nanos > 1_000_000 {
                let d = (nanos as f64) / 1_000_000f64;
                write!(f, "{:.3}ms", d);
            } else if nanos > 1_000 {
                let d = (nanos as f64) / 1_000f64;
                write!(f, "{:.3}us", d);
            } else {
                write!(f, "{}ns", nanos);
            }
        } else {
            let d = secs as f64 + nanos as f64 / 1_000_000_000f64;
            write!(f, "{:.3}s", d);
        }
        Ok(())
    }
}

fn main() {
    match run() {
        Ok(_) => {}
        Err(e) => { eprintln!("error: {:#?}", e) }
    }
}

#[derive(StructOpt, Debug, Clone)]
#[structopt(
global_settings(& [
structopt::clap::AppSettings::ColoredHelp,
structopt::clap::AppSettings::UnifiedHelpMessage
]),
)]
struct Config {
    #[structopt(short, parse(try_from_str = parse_duration), default_value("1s"))]
    /// time thread sleeps between pings
    pub interval: Duration,

    #[structopt(short, parse(try_from_str = parse_duration), default_value("5s"))]
    /// time-out of the ping
    pub timeout: Duration,

    #[structopt(short, parse(try_from_str = parse_duration), default_value("0s"))]
    /// interval that statistcs are printed out - 0 means do not print out
    pub stat_interval: Duration,

    #[structopt(parse(try_from_str = to_addr))]
    /// list of IPs or hostnames
    pub ips: Vec<IpAddr>,

    #[structopt(short = "v", parse(from_occurrences))]
    /// verbosity - good for testing.
    /// 1 will print successes otherwise is silent.
    pub verbose: usize,
}

/*
impl Config {
    fn from(args: &Vec<String>) -> ResultS<Config> {
        let mut interval = Duration::new(1, 0);
        let mut timeout = Duration::new(1, 0);
        let mut ips = vec![];
        let mut verbose = 0usize;
        let mut i = 1;
        loop {
            if args[i].starts_with("-") {
                println!("arg: {} {}", i, args[i]);
                match args[i].as_bytes() {
                    b"-i" => {
                        i = insure_next_index(&args, i, "-i requires interval duration in human format after it")?;
                        interval = parse_duration(&args[i])?;
                    }
                    b"-t" => {
                        i = insure_next_index(&args, i, "-t requires timeout duration in human format after it")?;
                        timeout = parse_duration(&args[i])?;
                    }
                    b"-v" => verbose = 1usize,
                    b"-vv" => verbose = 2usize,
                    b"-vvv" => verbose = 3usize,
                    b"-vvvv" => verbose = 4usize,
                    _ => return Err(anyhow!("option {} not understood", args[i])),
                }
                println!("after arg: {} {}", i, args[i]);
            } else {
                println!("no arg: {}", args[i]);

                let ip_addr = to_addr(&args[i])?;
                ips.push(ip_addr);
            }
            i += 1;
            if i >= args.len() {
                break;
            }
        }

        Ok(Config {
            interval,
            timeout,
            ips,
            verbose,
        })
    }
}

fn insure_next_index(args: &Vec<String>, i: usize, msg: &str) -> ResultS<usize> {
    if args.len() > i {
        return Ok(i + 1);
    } else {
        return Err(anyhow!(format!("Missing argument: {}", msg)));
    }
}
*/

struct Stats {
    work: AtomicU64,
    fail: AtomicU64,
    time_sum_us: AtomicU64,
    time_min_us: AtomicU64,
    time_max_us: AtomicU64,
}

impl Stats {
    pub fn new() -> Stats {
        Stats {
            work: AtomicU64::new(0),
            fail: AtomicU64::new(0),
            time_sum_us: AtomicU64::new(0),
            time_min_us: AtomicU64::new(std::u64::MAX),
            time_max_us: AtomicU64::new(0),
        }
    }
}

fn run() -> ResultS<()> {
    let args: Vec<String> = env::args().collect();
    let cfg: Config = Config::from_args();
    if cfg.verbose > 1 {
        println!("options: \n{:#?}", &cfg);
    }
    //let cfg = Config::from(&args)?;

    println!("starting....");

    let mut threads = vec![];
    let mut tracks = vec![];
    for ip in &cfg.ips {
        let ip = ip.clone();
        let verbose = cfg.verbose;
        let interval = cfg.interval;
        let timeout = cfg.timeout;
        let mut stats = Arc::new(Stats::new());

        tracks.push(stats.clone());
        let h = std::thread::spawn(move || {
            let mut seq_cnt = 0u16;
            loop {
                let now = Instant::now();
                let res = ping::ping(ip, Some(timeout),
                                     Some(64), Some(2112), Some(seq_cnt), None);
                let dur = now.elapsed();
                let st = SystemTime::now();
                seq_cnt = seq_cnt.wrapping_add(1);
                match res {
                    Ok(reply) => {
                        stats.work.fetch_add(1, Ordering::Relaxed);
                        let micros = (dur.as_nanos() / 1000) as u64;
                        stats.time_sum_us.fetch_add(micros, Ordering::Relaxed);
                        stats.time_min_us.fetch_min(micros, Ordering::Relaxed);
                        stats.time_max_us.fetch_max(micros, Ordering::Relaxed);
                        if verbose > 0 {
                            println!("[{}]  success for ip: {} in {}", format_rfc3339_millis(st), &ip, format_duration_mine(dur));
                            if verbose > 1 {
                                println!("\tident: {} seq_cnt {}  ", reply.0, reply.1);
                            }
                        }
                    }
                    Err(e) => {
                        stats.fail.fetch_add(1, Ordering::Relaxed);
                        if let Some(e) = e.downcast_ref::<std::io::Error>() {
                            match e.kind() {
                                std::io::ErrorKind::TimedOut => println!("[{}]  timeout for ip {} seq_cnt: {} after {}", format_rfc3339_millis(st), &ip, seq_cnt, format_duration_mine(dur)),
                                _ => println!("[{}]  io error for ip {}, error: {}", format_rfc3339_millis(st), &ip, e),
                            }
                        } else {
                            println!("[{}]  general error for ip {} of {:?}", format_rfc3339_millis(st), &ip, e);
                        }
                    }
                }
                std::thread::sleep(interval);
            }
        });
        threads.push(h);
    }


    let mut ip_list = vec![];
    for ip in &cfg.ips {
        let a = ip.clone();
        ip_list.push(a);
    }

    if cfg.stat_interval.as_millis() > 0 {
        let tracker_h = std::thread::spawn(move || {
            loop {
                std::thread::sleep(cfg.stat_interval);
                let st = SystemTime::now();
                for i in 0..cfg.ips.len() {
                    let ip = ip_list.get(i).unwrap();
                    let worked = tracks.get(i).unwrap().work.swap(0, Ordering::Relaxed);
                    let failed = tracks.get(i).unwrap().fail.swap(0, Ordering::Relaxed);
                    let sum = tracks.get(i).unwrap().time_sum_us.swap(0, Ordering::Relaxed);
                    let min = tracks.get(i).unwrap().time_min_us.swap(std::u64::MAX, Ordering::Relaxed);
                    let max = tracks.get(i).unwrap().time_max_us.swap(0, Ordering::Relaxed);
                    if worked > 0 {
                        let avg_ms = (sum as f64 / worked as f64) / 1000f64;
                        let min_ms = min as f64 / 1000f64;
                        let max_ms = max as f64 / 1000f64;
                        println!("[{}] ip: {}  worked: {}  failed: {}  times, avg: {:.3}ms  min: {:.3}ms  max: {:.3}ms",
                                 format_rfc3339_millis(st), &ip, worked, failed,
                                 avg_ms, min_ms, max_ms);
                    } else {
                        println!("[{}] ip: {}  worked: {}  failed: {}",
                                 format_rfc3339_millis(st), &ip, worked, failed);

                    }
                }
            }
        });
    } else {
        println!("no stats tracking started");
    }

    for h in threads {
        h.join();
    }
    Ok(())
}


fn to_addr(s: &str) -> ResultS<IpAddr> {
    if s.contains(":") {
        Ok(s.to_socket_addrs().with_context(|| format!("unknown host for \"{}\"", s))?.next().expect("no IP address found for host").ip())
    } else {
        let mut snew: String = String::from(s);
        snew.push_str(":22");
        Ok(snew.to_socket_addrs().with_context(|| format!("unknown host for \"{}\"", s))?.next().expect("no IP address found for host").ip())
    }
}