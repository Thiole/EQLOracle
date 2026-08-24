//! why: links a cast.begin to its later resisted/interrupted/fizzled/landed
//! line by proximity -- silent debuffs stay Unconfirmed, never guessed
//! landed. Warding is out of scope (never names a spell). Rank recovery
//! is an inherited gap: landed lines drop the rank numeral, so ranks
//! collide inside one resolution window until that's solved.

use std::collections::HashMap;

pub type Millis = i64;

/// What happened to a cast, once resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// why: confirmed by damage/heal, or a named state transition
    Landed,
    /// `<target> resisted your <spell>!`
    Resisted,
    /// `<spell> spell is interrupted.`
    Interrupted,
    /// `<spell> spell fizzles!`
    Fizzled,
    /// why: no terminal line arrived -- honest, not "probably landed"
    Unconfirmed,
}

impl Outcome {
    pub fn name(self) -> &'static str {
        match self {
            Outcome::Landed => "landed",
            Outcome::Resisted => "resisted",
            Outcome::Interrupted => "interrupted",
            Outcome::Fizzled => "fizzled",
            Outcome::Unconfirmed => "unconfirmed",
        }
    }
}

/// One resolved cast.
#[derive(Debug, Clone, Copy)]
pub struct Resolution {
    pub start_ms: Millis,
    pub end_ms: Millis,
    pub source: u32,
    pub spell: u32,
    pub outcome: Outcome,
}

/// A cast that started but hasn't resolved yet.
#[derive(Debug, Clone, Copy)]
struct Pending {
    start_ms: Millis,
    spell: u32,
}

/// why: generous timeout before force-closing Unconfirmed, unmeasured
pub const RESOLUTION_TIMEOUT_MS: Millis = 8_000;

/// why: one pending cast per source -- a second begin force-resolves the first
#[derive(Debug, Default)]
pub struct Resolver {
    open: HashMap<u32, Pending>,
    resolved: Vec<Resolution>,
}

impl Resolver {
    /// why: `spell` is an opaque caller key, only ever compared to itself
    pub fn begin(&mut self, ts: Millis, source: u32, spell: u32) {
        self.force_close(ts, source, Outcome::Unconfirmed);
        self.open.insert(
            source,
            Pending {
                start_ms: ts,
                spell,
            },
        );
    }

    /// why: only closes if `spell` matches what's actually open
    pub fn resolve(&mut self, ts: Millis, source: u32, spell: u32, outcome: Outcome) {
        if self.open.get(&source).is_some_and(|p| p.spell == spell) {
            self.close(ts, source, outcome);
        }
    }

    /// why: `spell` must be rank-stripped, see module doc on rank recovery
    pub fn confirm_landed(&mut self, ts: Millis, source: u32, spell: u32) {
        self.resolve(ts, source, spell, Outcome::Landed);
    }

    fn close(&mut self, ts: Millis, source: u32, outcome: Outcome) {
        if let Some(p) = self.open.remove(&source) {
            self.resolved.push(Resolution {
                start_ms: p.start_ms,
                end_ms: ts,
                source,
                spell: p.spell,
                outcome,
            });
        }
    }

    fn force_close(&mut self, ts: Millis, source: u32, outcome: Outcome) {
        if self.open.contains_key(&source) {
            self.close(ts, source, outcome);
        }
    }

    /// why: sweeps stale pending casts so a quiet caster never stays open
    pub fn expire(&mut self, now: Millis) {
        let stale: Vec<u32> = self
            .open
            .iter()
            .filter(|(_, p)| now - p.start_ms > RESOLUTION_TIMEOUT_MS)
            .map(|(&src, _)| src)
            .collect();
        for src in stale {
            self.close(now, src, Outcome::Unconfirmed);
        }
    }

    /// Resolved casts accumulated so far.
    pub fn resolved(&self) -> &[Resolution] {
        &self.resolved
    }

    /// why: keeps this struct from becoming an unbounded session log
    pub fn drain_resolved(&mut self) -> Vec<Resolution> {
        std::mem::take(&mut self.resolved)
    }
}
