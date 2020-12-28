use structopt::StructOpt;
use std::time::Duration;
use std::net::{IpAddr, ToSocketAddrs};
use humantime::parse_duration;
use std::str::FromStr;
use anyhow::{anyhow,Context};

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
    pub ips: Vec<IpAddr>,

    #[structopt(short = "v", parse(from_occurrences))]
    /// verbosity - good for testing.
    /// 1 will print successes otherwise is silent.
    pub verbose: usize,
}

fn to_addr(s: &str) -> ResultS<IpAddr> {
    match s.to_socket_addrs() {
        Ok(mut ip) => {
            if let Some(x) = ip.next() {
                println!("to addr ip: {}", x.ip());
                Ok(x.ip())
            } else {
                Err(anyhow!("error in look or address interpretation for \"{}\"", s))
            }
        }
        Err(e) => {
            match IpAddr::from_str(s) {
                Ok(ip) => {
                    Ok(ip)
                }
                Err(e) => {
                    let mut snew: String = String::from(s);
                    snew.push_str(":0");
                    let mut x = snew
                        .to_socket_addrs()
                        .with_context(|| format!("unknown host or IP for \"{}\" using faked port 22", s))?
                        .next().expect("no IP address found for host").ip();
                    println!("to addr (with :0 added) ip: {}", &x);
                    Ok(x)
                    // Err(anyhow!("tried to parse \"{}\" as ip address and cannot, error: {}", s, e))
                }
            }
        }
    }
}
