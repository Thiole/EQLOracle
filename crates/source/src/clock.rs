//! Time as an injected dependency. Nothing downstream calls `Instant::now()`;
//! CI enforces it.
//!
//! Design notes: `docs/design/sources.md`

use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Milliseconds since the Unix epoch.
pub type Millis = i64;

pub trait Clock: Send + Sync {
    fn now_ms(&self) -> Millis;

    fn now_secs(&self) -> i64 {
        self.now_ms().div_euclid(1000)
    }
}

/// Wall clock. The only impl that touches the OS.
#[derive(Default, Debug, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> Millis {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            // why: never panic a log parser over a dead BIOS battery
            .unwrap_or(0)
    }
}

/// Clock that only moves when told. Drives replay and deterministic tests.
#[derive(Debug)]
pub struct VirtualClock {
    ms: AtomicI64,
}

impl VirtualClock {
    pub fn new(start_ms: Millis) -> Self {
        VirtualClock {
            ms: AtomicI64::new(start_ms),
        }
    }

    pub fn at_unix_secs(s: i64) -> Self {
        VirtualClock::new(s * 1000)
    }

    pub fn advance_ms(&self, d: Millis) {
        self.ms.fetch_add(d, Ordering::Relaxed);
    }

    /// Jump forward to an absolute time. Never moves backwards.
    pub fn set_at_least(&self, ms: Millis) {
        let mut cur = self.ms.load(Ordering::Relaxed);
        while ms > cur {
            match self
                .ms
                .compare_exchange_weak(cur, ms, Ordering::Relaxed, Ordering::Relaxed)
            {
                Ok(_) => return,
                Err(actual) => cur = actual,
            }
        }
    }
}

impl Default for VirtualClock {
    fn default() -> Self {
        VirtualClock::new(0)
    }
}

impl Clock for VirtualClock {
    fn now_ms(&self) -> Millis {
        self.ms.load(Ordering::Relaxed)
    }
}

impl<T: Clock + ?Sized> Clock for std::sync::Arc<T> {
    fn now_ms(&self) -> Millis {
        (**self).now_ms()
    }
}

impl<T: Clock + ?Sized> Clock for &T {
    fn now_ms(&self) -> Millis {
        (**self).now_ms()
    }
}
