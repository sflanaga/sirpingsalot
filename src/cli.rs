use structopt::StructOpt;
use std::time::Duration;
use std::net::{IpAddr, ToSocketAddrs};
use humantime::parse_duration;
use std::str::FromStr;
use anyhow::{anyhow,Context};
use std::fmt;
use log::LevelFilter;

type ResultS<T> = std::result::Result<T, anyhow::Error>;

#[derive(StructOpt, Debug, Clone)]
#[structopt(
global_settings(& [
structopt::clap::AppSettings::ColoredHelp,
structopt::clap::AppSettings::UnifiedHelpMessage
]),
)]
pub struct Config {
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
    pub ips: Vec<HostInfo>,

    #[structopt(short)]
    /// write packet details if anything seems "unusual"
    pub raw_write_odd: bool,

    #[structopt(short="L", long, parse(try_from_str = to_log_level), default_value("info"))]
    /// log level
    pub log_level: LevelFilter,

    #[structopt(short="I", long, default_value("11000"))]
    /// log level
    pub ident_base: u16,

}

pub fn to_addr(s: &str) -> ResultS<HostInfo> {
    match s.to_socket_addrs() {
        Ok(mut ip) => {
            if let Some(x) = ip.next() {
                println!("to addr ip: {}", x.ip());
                Ok(HostInfo::new(None,x.ip()))
            } else {
                Err(anyhow!("error in look or address interpretation for \"{}\"", s))
            }
        }
        Err(e) => {
            match IpAddr::from_str(s) {
                Ok(ip) => {
                    Ok(HostInfo::new(None,ip))
                }
                Err(e) => {
                    let mut snew: String = String::from(s);
                    snew.push_str(":0");
                    let mut x = snew
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
