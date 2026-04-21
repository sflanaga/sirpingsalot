use clap::Parser;
use std::time::Duration;
use std::net::{IpAddr, ToSocketAddrs};
use humantime::parse_duration;
use std::str::FromStr;
use anyhow::{anyhow,Context};
use std::fmt;
use log::{debug, LevelFilter};

type ResultS<T> = std::result::Result<T, anyhow::Error>;

#[derive(Parser, Debug, Clone)]
#[allow(dead_code)]
pub struct Config {
    #[arg(short, value_parser = parse_duration, default_value = "1s")]
    /// time thread sleeps between pings
    pub interval: Duration,

    #[arg(short, value_parser = parse_duration, default_value = "5s")]
    /// time-out of the ping
    pub timeout: Duration,

    #[arg(short, value_parser = parse_duration, default_value = "0s")]
    /// interval that statistcs are printed out - 0 means do not print out
    pub stat_interval: Duration,

    #[arg(value_parser = to_addr)]
    /// list of IPs or hostnames
    pub ips: Vec<HostInfo>,

    #[arg(short)]
    /// write packet details if anything seems "unusual"
    pub raw_write_odd: bool,

    #[arg(short = 'L', long, value_parser = to_log_level, default_value = "info")]
    /// log level
    pub log_level: LevelFilter,

    #[arg(short = 'I', long, default_value = "11000")]
    /// log level
    pub ident_base: u16,

    #[arg(short = 'R')]
    /// reset stats after each print interval (default: print cumulative stats since start)
    pub reset_stats: bool,

}

pub fn to_addr(s: &str) -> ResultS<HostInfo> {
    match s.to_socket_addrs() {
        Ok(mut ip) => {
            if let Some(x) = ip.next() {
                debug!("to addr ip: {}", x.ip());
                Ok(HostInfo::new(None,x.ip()))
            } else {
                Err(anyhow!("error in look or address interpretation for \"{}\"", s))
            }
        }
        Err(_e) => {
            match IpAddr::from_str(s) {
                Ok(ip) => {
                    Ok(HostInfo::new(None,ip))
                }
                Err(_e) => {
                    let mut snew: String = String::from(s);
                    snew.push_str(":0");
                    let x = snew
                        .to_socket_addrs()
                        .with_context(|| format!("unknown host or IP for \"{}\" using faked port 22", s))?
                        .next().expect("no IP address found for host").ip();
                    Ok(HostInfo::new(Some(String::from(s)),x))
                    // Err(anyhow!("tried to parse \"{}\" as ip address and cannot, error: {}", s, e))
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct HostInfo {
    pub host: Option<String> ,
    pub ip: IpAddr,
}

impl HostInfo {
    pub fn new(host:Option<String>, ip: IpAddr) -> HostInfo {
        HostInfo {
            host,
            ip,
        }
    }
}

impl fmt::Display for HostInfo {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.host.is_none() {
            write!(f, "{}", self.ip)
        } else {
            write!(f, "{}({})", self.host.as_ref().unwrap(), self.ip)
        }
    }
}

pub fn to_log_level(s: &str) -> anyhow::Result<LevelFilter, anyhow::Error> {
    match s {
        "off" | "o" => Ok(LevelFilter::Off),
        "error" | "e"  => Ok(LevelFilter::Error),
        "warn" | "w" => Ok(LevelFilter::Warn),
        "info" | "i" => Ok(LevelFilter::Info),
        "debug" | "d" => Ok(LevelFilter::Debug),
        "trace" | "t" => Ok(LevelFilter::Trace),
        _ => Err(anyhow::anyhow!("Error for log level: must be one of off, o, error, e, warn, w, info, i, debug, d, trace, t but got {}", &s))
    }
}
