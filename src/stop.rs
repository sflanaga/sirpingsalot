use std::sync::atomic::AtomicBool;
use std::time::Duration;
use std::sync::{Condvar, Arc, Mutex};


#[derive(Debug, Clone)]
pub struct Stop {
    stop: Arc<Mutex<bool>>,
    cond: Arc<Condvar>
}

impl Stop {
    pub fn new() -> Stop {
        let n = Stop {
            stop: Arc::new(Mutex::new(false)),
            cond: Arc::new(Condvar::new()),
        };
        let mut ctrl_n = n.clone();
        ctrlc::set_handler(move || {
            println!("told to stop via \"ctrl-c\"");
            ctrl_n.signal();
        }).expect("Error setting Ctrl-C handler");
        n
    }

    pub fn sleep(&self, time: Duration) -> bool {
        let lock = self.stop.lock().unwrap();
        if *lock {
            true
        } else {
            let result = self.cond.wait_timeout(lock, time).unwrap();
            if *result.0 {
                true
            } else {
                false
            }
        }
    }

    fn signal(&mut self) {
        let mut lock = self.stop.lock().unwrap();
        *lock = true;
        self.cond.notify_all();
    }
}