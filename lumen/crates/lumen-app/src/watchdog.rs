//! Stall detection for show mode.
//!
//! The render loop beats a heartbeat every frame. A watchdog thread checks it; if the loop has
//! not beaten for longer than the stall limit the configured action runs (log, or relaunch the
//! show). This cannot fix a hung GPU driver, but it does turn a silent frozen projector into a
//! logged event and, when asked, a restarted show.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Shared "last frame" timestamp in milliseconds since the heartbeat was created.
#[derive(Debug, Clone)]
pub struct Heartbeat {
    start: Instant,
    last_ms: Arc<AtomicU64>,
}

impl Default for Heartbeat {
    fn default() -> Self {
        Self::new()
    }
}

impl Heartbeat {
    /// Creates a heartbeat that has just beaten.
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
            last_ms: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Records a frame.
    pub fn beat(&self) {
        self.last_ms
            .store(self.start.elapsed().as_millis() as u64, Ordering::Relaxed);
    }

    /// Time since the last beat.
    pub fn since_last(&self) -> Duration {
        let now = self.start.elapsed().as_millis() as u64;
        Duration::from_millis(now.saturating_sub(self.last_ms.load(Ordering::Relaxed)))
    }
}

/// True when `since_last` exceeds the limit.
pub fn stalled(since_last: Duration, limit: Duration) -> bool {
    since_last > limit
}

/// Background checker.
#[derive(Debug)]
pub struct Watchdog {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Watchdog {
    /// Starts checking `heartbeat` every `limit / 4`; calls `on_stall` once per stall.
    pub fn spawn(
        heartbeat: Heartbeat,
        limit: Duration,
        on_stall: impl Fn(Duration) + Send + 'static,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = Arc::clone(&stop);
        let thread = std::thread::Builder::new()
            .name("lumen-watchdog".into())
            .spawn(move || {
                let tick = (limit / 4).max(Duration::from_millis(10));
                let mut reported = false;
                while !stop_flag.load(Ordering::Relaxed) {
                    std::thread::sleep(tick);
                    let since = heartbeat.since_last();
                    if stalled(since, limit) {
                        if !reported {
                            on_stall(since);
                            reported = true;
                        }
                    } else {
                        reported = false;
                    }
                }
            })
            .ok();
        Self { stop, thread }
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Relaunches the current executable with the same arguments and exits this process.
///
/// Used by `lumen show --restart-on-stall`. Returns only if the relaunch failed.
pub fn relaunch_self() -> std::io::Error {
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => return e,
    };
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    match std::process::Command::new(exe).args(args).spawn() {
        Ok(_) => std::process::exit(3),
        Err(e) => e,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn stall_rule() {
        assert!(!stalled(
            Duration::from_millis(100),
            Duration::from_millis(200)
        ));
        assert!(stalled(
            Duration::from_millis(300),
            Duration::from_millis(200)
        ));
    }

    #[test]
    fn heartbeat_measures_gaps() {
        let hb = Heartbeat::default();
        hb.beat();
        assert!(hb.since_last() < Duration::from_millis(500));
        std::thread::sleep(Duration::from_millis(30));
        assert!(hb.since_last() >= Duration::from_millis(25));
        hb.beat();
        assert!(hb.since_last() < Duration::from_millis(25));
    }

    #[test]
    fn watchdog_fires_once_per_stall_and_recovers() {
        let hb = Heartbeat::new();
        let fired = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&fired);
        let dog = Watchdog::spawn(hb.clone(), Duration::from_millis(80), move |d| {
            sink.lock().unwrap().push(d);
        });
        // Keep beating: no stall.
        for _ in 0..5 {
            hb.beat();
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(fired.lock().unwrap().is_empty());
        // Stop beating: exactly one report.
        std::thread::sleep(Duration::from_millis(250));
        assert_eq!(fired.lock().unwrap().len(), 1);
        // Recover, then stall again: a second report.
        hb.beat();
        std::thread::sleep(Duration::from_millis(30));
        std::thread::sleep(Duration::from_millis(250));
        assert_eq!(fired.lock().unwrap().len(), 2);
        drop(dog);
    }
}
