//! Resolving what happened to a spell cast.
//!
//! `cast.begin` only proves an attempt started. What happens next comes from
//! a disjoint set of rules -- resisted, interrupted, fizzled, or (via a
//! damage/heal line, or a named state change like mesmerized/charmed)
//! landed -- and nothing in the log links any of them back to the cast that
//! produced them except proximity: same source, same spell, shortly after.
//! This module is that link.
//!
//! The state most parsers skip is [`Outcome::Unconfirmed`]. A meaningful
//! slice of debuffs print no result text at all, win or lose -- memory
//! blur, pacify, lull, and unresistable stat debuffs like the Tash line all
//! measured at zero resisted-lines and zero landed-lines against the
//! reference log, over a thousand casts combined. Coercing that silence
//! into "landed" would be exactly the kind of confidently wrong number
//! `FOUNDATION.md` warns about -- so it isn't. A cast this module can't
//! account for ends in `Unconfirmed`, not in a guess.
//!
//! Deliberately out of scope: warding. `spell.warded` in the rule pack only
//! ever fires as `"<attacker> tries to cast a spell on you, but you are
//! protected."` -- every one of 7,529 occurrences in the reference log is
//! "on you", never a third-person form, and the line never names a spell.
//! That makes it a player buff-state signal, not something attributable to
//! a specific outgoing cast, so it has no place in this resolver.
//!
//! Rank recovery is a hard dependency this module inherits rather than
//! solves: `resolve`'s terminal-text callers (resisted/interrupted/fizzled)
//! carry the same rank-numbered spell name `begin` does, so they match
//! exactly, but `confirm_landed` via a damage line does not -- landed lines
//! drop the rank numeral (see `BACKLOG.md`, "Rank recovery"). Until that's
//! solved, the caller must pass a rank-stripped key on both sides of
//! `confirm_landed`, which means two different ranks of the same spell in
//! flight from one caster inside one resolution window are indistinguishable
//! here. Rare, not impossible, and worth revisiting once rank recovery lands.

use std::collections::HashMap;

pub type Millis = i64;

/// What happened to a cast, once resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Confirmed by a damage/heal line, or a named state transition (mez,
    /// charm) attributed to this cast.
    Landed,
    /// `<target> resisted your <spell>!`
    Resisted,
    /// `<spell> spell is interrupted.`
    Interrupted,
    /// `<spell> spell fizzles!`
    Fizzled,
    /// No terminal line arrived before the source's next cast, or the
    /// resolution window elapsed. Not "probably landed" -- see the module
    /// doc. This is the honest answer for memory blur, pacify, lull, and
    /// the Tash line.
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

/// How long an unresolved cast waits before being force-closed as
/// `Unconfirmed`. Generous: cast times run several seconds, and a resist or
/// fizzle line can lag a tick behind the swing that caused it. Chosen to
/// match `PET_MATCH_WINDOW_MS`'s order of magnitude, not measured against
/// the reference log the way that constant was -- revisit once there's a
/// coverage run to check it against.
pub const RESOLUTION_TIMEOUT_MS: Millis = 8_000;

/// Tracks one pending cast per source and resolves it against later lines.
///
/// A source can only be casting one spell at a time -- bard singing while
/// twisting is the one real exception in EQ, and it is out of scope here,
/// the same call `FOUNDATION.md` makes elsewhere about not modelling what
/// the log can't distinguish. A second `begin` from the same source is
/// proof the first one ended without a terminal line (the client would not
/// let a new cast start otherwise), so it force-resolves the old one as
/// `Unconfirmed` before opening the new one.
#[derive(Debug, Default)]
pub struct Resolver {
    open: HashMap<u32, Pending>,
    resolved: Vec<Resolution>,
}

impl Resolver {
    /// A new cast started. `spell` is an opaque caller-assigned key -- an
    /// interned name, an `AbilityId`, whatever the caller already has. This
    /// module never compares it to anything but itself.
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

    /// A terminal line arrived (resisted, interrupted, fizzled). Only closes
    /// the pending cast if `spell` matches what's open for `source` -- a
    /// resist line for a different spell than the one currently pending
    /// must not be misfiled against it.
    pub fn resolve(&mut self, ts: Millis, source: u32, spell: u32, outcome: Outcome) {
        if self.open.get(&source).is_some_and(|p| p.spell == spell) {
            self.close(ts, source, outcome);
        }
    }

    /// A damage/heal line or a named state transition (mez, charm) confirms
    /// landing. See the module doc's note on rank recovery for what `spell`
    /// must be here.
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

    /// Sweep every pending cast older than [`RESOLUTION_TIMEOUT_MS`] into
    /// `Unconfirmed`. Call periodically (the same cadence as encounter
    /// expiry) so a caster who goes quiet -- logs off, zones, the log ends
    /// -- doesn't leave a cast open forever.
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

    /// Drain resolved casts, e.g. to push into a store. Keeps this struct
    /// from becoming an unbounded log of every cast for the session.
    pub fn drain_resolved(&mut self) -> Vec<Resolution> {
        std::mem::take(&mut self.resolved)
    }
}
