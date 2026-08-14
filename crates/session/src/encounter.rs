//! Encounter tracking, per-source damage, and time-to-kill estimation.
//!
//! Design notes: `docs/design/session.md`

use crate::rolling::Rolling;
use eqlp_source::Millis;
use std::collections::HashMap;

use crate::fold_key as key;

pub const DEFAULT_TIMEOUT_MS: Millis = 12_000;
pub const DEFAULT_WINDOW_MS: Millis = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndReason {
    /// A death line named this target. Duration is exact.
    Slain,
    /// No damage for the timeout. Duration is an upper bound.
    Timeout,
    /// Explicitly closed (zone change, log rotation, replay end).
    Closed,
}

#[derive(Debug, Clone)]
pub struct Encounter {
    pub target: String,
    pub start_ms: Millis,
    pub last_ms: Millis,
    pub end_ms: Option<Millis>,
    pub end_reason: Option<EndReason>,
    pub total: u64,
    /// Damage by source name. Charmed pets appear under their own mob name.
    pub by_source: HashMap<String, Rolling>,
    /// Set when damage continued arriving after a death line for this name,
    /// which means more than one mob shares it.
    pub merged_suspected: bool,
}

impl Encounter {
    fn new(target: String, ts: Millis) -> Self {
        Encounter {
            target,
            start_ms: ts,
            last_ms: ts,
            end_ms: None,
            end_reason: None,
            total: 0,
            by_source: HashMap::new(),
            merged_suspected: false,
        }
    }

    pub fn duration_ms(&self, now: Millis) -> Millis {
        self.end_ms.unwrap_or(now) - self.start_ms
    }

    pub fn is_open(&self) -> bool {
        self.end_ms.is_none()
    }

    /// Live DPS over the trailing window, all sources.
    pub fn dps(&mut self, now: Millis) -> f64 {
        self.by_source.values_mut().map(|r| r.dps(now)).sum()
    }

    /// Whole-fight DPS.
    pub fn dps_overall(&self, now: Millis) -> f64 {
        let d = self.duration_ms(now).max(crate::rolling::MIN_DIVISOR_MS);
        self.total as f64 / (d as f64 / 1000.0)
    }

    pub fn dps_by_source(&mut self, now: Millis) -> Vec<(String, f64, u64)> {
        let mut v: Vec<_> = self
            .by_source
            .iter_mut()
            .map(|(k, r)| (k.clone(), r.dps(now), r.total))
            .collect();
        v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        v
    }
}

/// Observed kill damage per mob name, used as an HP estimate for TTK.
/// `Timeout` fights are excluded. See `docs/design/session.md`.
#[derive(Debug, Default)]
pub struct HpModel {
    kills: HashMap<String, Vec<u64>>,
}

impl HpModel {
    pub fn record_kill(&mut self, target: &str, total_damage: u64) {
        let v = self.kills.entry(target.to_string()).or_default();
        v.push(total_damage);
        if v.len() > 64 {
            v.remove(0);
        }
    }

    /// Median observed kill damage, once there is enough to mean anything.
    pub fn estimate(&self, target: &str) -> Option<u64> {
        let v = self.kills.get(target).or_else(|| self.kills.get(&key(target)))?;
        if v.len() < 3 {
            return None;
        }
        let mut s = v.clone();
        s.sort_unstable();
        Some(s[s.len() / 2])
    }

    pub fn samples(&self, target: &str) -> usize {
        self.kills.get(&key(target)).map_or(0, |v| v.len())
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Ttk {
    /// Seconds remaining at the current rate.
    Seconds(f64),
    /// Known mob, but current DPS is ~0, so it is not dying.
    Stalled,
    /// Fewer than 3 recorded kills of this name.
    NoBaseline,
    /// Already past the observed median — the estimate has been exceeded.
    Overrun,
}

pub struct Tracker {
    open: HashMap<String, Encounter>,
    pub done: Vec<Encounter>,
    pub hp: HpModel,
    timeout_ms: Millis,
    window_ms: Millis,
    max_done: usize,
}

impl Default for Tracker {
    fn default() -> Self {
        Tracker::new(DEFAULT_TIMEOUT_MS, DEFAULT_WINDOW_MS)
    }
}

impl Tracker {
    pub fn new(timeout_ms: Millis, window_ms: Millis) -> Self {
        Tracker {
            open: HashMap::new(),
            done: Vec::new(),
            hp: HpModel::default(),
            timeout_ms,
            window_ms,
            max_done: 4096,
        }
    }

    pub fn damage(&mut self, ts: Millis, source: &str, target: &str, amount: u64) {
        let w = self.window_ms;
        let k = key(target);
        let e = self
            .open
            .entry(k)
            .or_insert_with(|| Encounter::new(target.to_string(), ts));
        e.last_ms = ts;
        e.total += amount;
        e.by_source
            .entry(source.to_string())
            .or_insert_with(|| Rolling::new(w))
            .push(ts, amount);
    }

    /// A death line naming `target`.
    pub fn death(&mut self, ts: Millis, target: &str) {
        if let Some(mut e) = self.open.remove(&key(target)) {
            e.end_ms = Some(ts);
            e.end_reason = Some(EndReason::Slain);
            if e.total > 0 {
                self.hp.record_kill(&key(target), e.total);
            }
            self.retire(e);
        }
    }

    /// Call every UI tick. Closes fights that have gone quiet.
    pub fn tick(&mut self, now: Millis) {
        let stale: Vec<String> = self
            .open
            .iter()
            .filter(|(_, e)| now - e.last_ms > self.timeout_ms)
            .map(|(k, _)| k.clone())
            .collect();
        for k in stale {
            if let Some(mut e) = self.open.remove(&k) {
                e.end_ms = Some(e.last_ms);
                e.end_reason = Some(EndReason::Timeout);
                self.retire(e);
            }
        }
    }

    fn retire(&mut self, e: Encounter) {
        self.done.push(e);
        if self.done.len() > self.max_done {
            self.done.remove(0);
        }
    }

    pub fn open_encounters(&mut self) -> impl Iterator<Item = &mut Encounter> {
        self.open.values_mut()
    }

    /// Number of fights currently in progress.
    pub fn open_count(&self) -> usize {
        self.open.len()
    }

    pub fn get(&mut self, target: &str) -> Option<&mut Encounter> {
        self.open.get_mut(&key(target))
    }

    /// Time to kill, from observed kill damage and current DPS.
    pub fn ttk(&mut self, target: &str, now: Millis) -> Ttk {
        let est = match self.hp.estimate(target) {
            Some(v) => v,
            None => return Ttk::NoBaseline,
        };
        let e = match self.open.get_mut(&key(target)) {
            Some(e) => e,
            None => return Ttk::NoBaseline,
        };
        if e.total >= est {
            return Ttk::Overrun;
        }
        let dps = e.dps(now);
        if dps < 1.0 {
            return Ttk::Stalled;
        }
        Ttk::Seconds((est - e.total) as f64 / dps)
    }

    /// Zone change, log rotation, or replay end.
    pub fn close_all(&mut self, ts: Millis) {
        let keys: Vec<String> = self.open.keys().cloned().collect();
        for k in keys {
            if let Some(mut e) = self.open.remove(&k) {
                e.end_ms = Some(ts.max(e.last_ms));
                e.end_reason = Some(EndReason::Closed);
                self.retire(e);
            }
        }
    }
}
