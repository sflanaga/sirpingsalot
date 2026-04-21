#![allow(unused_imports, unused_variables, unused_mut, unused_parens, dead_code)]

use std::time::{Duration, SystemTime};
use std::fmt;
use socket2::SockAddr;
use std::fmt::{Display, Formatter};
use std::net::IpAddr;
use crate::stop::Stop;
use log::LevelFilter;
use humantime::format_rfc3339_millis;
use std::sync::atomic::{AtomicBool, Ordering};
use std::io::IsTerminal;

/// Set to true when stderr is a terminal; used to gate status-line tricks.
pub static STDERR_IS_TERMINAL: AtomicBool = AtomicBool::new(false);

pub fn sleep_until_next_interval_on(stop: &mut Stop, interval: Duration) -> bool {
    let now = SystemTime::now();
    let ep_dur = now.duration_since(SystemTime::UNIX_EPOCH).expect("UNIX_EPOCH should always be less than now");

    let this_sleep_time = (ep_dur.as_nanos() / interval.as_nanos() + 1) * interval.as_nanos() - ep_dur.as_nanos();
    let sleep_time = Duration::from_nanos(this_sleep_time as u64);

    // we do not care about the spurious wake up... well, not that much anyway
    stop.sleep(sleep_time)
}

/// Like `sleep_until_next_interval_on` but also wakes early when `trigger` is set.
/// Returns `(stopped, triggered_early)`.  Polls in 50ms chunks so the trigger
/// is noticed within ~50ms.
pub fn sleep_until_next_interval_or_trigger(
    stop: &mut Stop,
    interval: Duration,
    trigger: &AtomicBool,
) -> (bool, bool) {
    let ep_dur = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("UNIX_EPOCH should always be less than now");
    let next_ns = (ep_dur.as_nanos() / interval.as_nanos() + 1) * interval.as_nanos();
    let poll = Duration::from_millis(50);
    loop {
        if trigger.swap(false, Ordering::Relaxed) {
            return (false, true);
        }
        let now_ns = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH).unwrap()
            .as_nanos();
        if now_ns >= next_ns {
            return (false, false);
        }
        let remaining = Duration::from_nanos((next_ns - now_ns) as u64);
        if stop.sleep(remaining.min(poll)) {
            return (true, false);
        }
    }
}


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
                write!(f, "{:.3}ms", d)?;
            } else if nanos > 1_000 {
                let d = (nanos as f64) / 1_000f64;
                write!(f, "{:.3}us", d)?;
            } else {
                write!(f, "{}ns", nanos)?;
            }
        } else {
            let d = secs as f64 + nanos as f64 / 1_000_000_000f64;
            write!(f, "{:.3}s", d)?;
        }
        Ok(())
    }
}


pub struct SockAddrWrap<'a> {
    pub wrap: &'a SockAddr
}


impl Display for SockAddrWrap<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        if let Some(ip) = self.wrap.as_socket_ipv4() {
            write!(f, "ipv4: {}", ip)
        } else if let Some(ip) = self.wrap.as_socket_ipv6() {
            write!(f, "ipv6: {}", ip)
        } else {
            write!(f, "{:?}", &self.wrap)
        }
    }
}

pub fn init_log(level: LevelFilter) {
    STDERR_IS_TERMINAL.store(std::io::stderr().is_terminal(), Ordering::Relaxed);

    let mut builder = env_logger::Builder::new();

    use env_logger::fmt::Formatter;
    use std::io::Write;

    // builder.format(|buf, record| {
    //     writeln!(buf, "{} [{:4}] [{}:{}] {:>5}: {} ", format_rfc3339_millis(SystemTime::now()),
    //              std::thread::current().name().or(Some("unknown")).unwrap(),
    //              record.file().unwrap(),
    //              record.line().unwrap(),
    //              record.level(),
    //              record.args())
    // });
    builder.format(|buf, record| {
        use std::io::Write;
        if STDERR_IS_TERMINAL.load(Ordering::Relaxed) {
            write!(buf, "\r{:80}\r", "")?;
        }
        writeln!(buf, "{} [{:4}] {:>5} {} ", format_rfc3339_millis(SystemTime::now()),
                 std::thread::current().name().or(Some("unknown")).unwrap(),
                 record.level(),
                 record.args())
    });
    builder.filter_level(level);
    builder.init();


}

