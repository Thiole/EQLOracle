//! Sliding-window damage accumulator.
//!
//! Design notes: `docs/design/session.md`

use eqlp_source::Millis;
use std::collections::VecDeque;

/// Minimum divisor, to stop early-fight DPS spiking.
pub const MIN_DIVISOR_MS: Millis = 1500;

#[derive(Debug, Clone)]
pub struct Rolling {
    window_ms: Millis,
    ev: VecDeque<(Millis, u64)>,
    sum: u64,
    /// First event ever seen, for the ramp-in divisor.
    first_ms: Option<Millis>,
    pub total: u64,
    pub count: u64,
}

impl Rolling {
    pub fn new(window_ms: Millis) -> Self {
        Rolling {
            window_ms: window_ms.max(1000),
            ev: VecDeque::with_capacity(256),
            sum: 0,
            first_ms: None,
            total: 0,
            count: 0,
        }
    }

    pub fn push(&mut self, ts: Millis, amount: u64) {
        self.first_ms.get_or_insert(ts);
        self.ev.push_back((ts, amount));
        self.sum += amount;
        self.total += amount;
        self.count += 1;
        self.evict(ts);
    }

    /// Drop events outside the window. Call from the UI tick as well as on
    /// push. Window is half-open `(now - width, now]`.
    pub fn evict(&mut self, now: Millis) {
        let cutoff = now - self.window_ms;
        while let Some(&(t, a)) = self.ev.front() {
            if t <= cutoff {
                self.sum -= a;
                self.ev.pop_front();
            } else {
                break;
            }
        }
    }

    /// Damage per second over the trailing window.
    pub fn dps(&mut self, now: Millis) -> f64 {
        self.evict(now);
        let first = match self.first_ms {
            Some(f) => f,
            None => return 0.0,
        };
        let elapsed = now - first;
        let divisor = if elapsed >= self.window_ms {
            self.window_ms
        } else {
            elapsed.max(MIN_DIVISOR_MS)
        };
        self.sum as f64 / (divisor as f64 / 1000.0)
    }

    /// Damage per second across the whole fight, not just the window.
    pub fn dps_overall(&self, now: Millis) -> f64 {
        let first = match self.first_ms {
            Some(f) => f,
            None => return 0.0,
        };
        let d = (now - first).max(MIN_DIVISOR_MS);
        self.total as f64 / (d as f64 / 1000.0)
    }

    /// Events currently retained. Bounded by the window.
    pub fn buffered(&self) -> usize {
        self.ev.len()
    }

    pub fn window_ms(&self) -> Millis {
        self.window_ms
    }
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
}
