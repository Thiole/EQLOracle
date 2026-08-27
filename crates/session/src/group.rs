//! Dynamic, decaying belief about who's currently grouped with the log
//! owner. Unlike `Kind::Player` (permanent, monotonic once chat/pet
//! proven), real group membership can change at any moment with no log
//! signal at all -- someone can leave, and a name that once shared a
//! target was often never really in the party to begin with (a charmed
//! mob, a stranger farming the same public spawn). This tracker requires
//! reinforcement to stay "currently grouped", and fades if reinforcement
//! stops -- the opposite of `Entities`' own "evidence promotes, never
//! demotes" model, deliberately, because the underlying real-world fact
//! this tracks (who's with me right now) is itself not monotonic.
//!
//! Two reinforcement channels, genuinely different confidence levels:
//! - `reinforce_weak` (shared-target damage -- someone else hit the same
//!   mob "You" did): needs the same name to co-occur across >=
//!   `MIN_SESSIONS` real, gap-separated occasions before it counts at
//!   all. Measured against a real 245MB reference log before picking
//!   that gate, in two passes: a session-count of 2 (any later,
//!   gap-separated recurrence at all) still left 208 of 1,788 names
//!   qualifying, dominated by a recurring farmable charm-mob camp --
//!   those names visibly recur across many *real* sessions too, since
//!   the underlying game mechanic (revisit the same spot repeatedly) is
//!   identical in shape to a real standing group. The actual gap in that
//!   log's own session-count histogram was much further out: only 5 of
//!   1,788 names ever crossed 5 sessions at all (the 3 real standing
//!   groupmates at 11-16 sessions each, plus "unknown" -- a resolution
//!   artifact, not evidence -- and one recurring raid miniboss, caught
//!   instead by `eqlp-app`'s `is_known_npc_name` guard). `MIN_SESSIONS`
//!   is set to that measured gap, not the smallest value that technically works.
//! - `reinforce_strong` (a Quick Buff group-cast landing): the game
//!   itself scopes this to "all valid group and raid targets in range"
//!   (EQ Legends' own AA description) -- one confirmed landing is
//!   enough, no session gate needed. Confirmed against the same log: a
//!   real activation's landing burst named the log's own real standing
//!   groupmates, nothing else -- once `eqlp-app`'s own corroboration
//!   requirement (the same buff text must ALSO have just landed on "You")
//!   is tight enough. First cut wasn't: matching on any shared text let
//!   a widely-cast common spell (real example: "Center", a self-heal
//!   independently cast by 326 distinct entities across the log)
//!   "corroborate" by pure chance; fixed by also requiring the text
//!   itself be a recognized beneficial buff-flavor line, not an
//!   arbitrary spell-name landing.
//!
//! **A currently-charmed pet under a real party member's control counts
//! too, deliberately** -- neither channel excludes charm state.
//! Confirmed directly in the reference log (a charm-mob camp in The
//! Ruins of Old Guk, group-wide buffs landing on Keber/Kenobtik in the
//! exact same burst as the log's own real groupmates): while charmed,
//! it really is attacking the same targets and receiving the same buffs
//! as the rest of the group, so it really is "with them" for as long as
//! that holds -- excluding it would just be wrong, not more careful. An
//! earlier version of this tracker's own predecessor (the one-shot
//! permanent promotion this replaced) DID need a charm guard, because a
//! temporary ally becoming a PERMANENT one was the actual bug; that
//! constraint doesn't carry over here, since nothing this tracker grants
//! is permanent -- once the charm actually ends, the pet simply stops
//! co-occurring with the group and decays back out within `GROUP_TTL_MS`
//! on its own, no special-casing required. Real, separately fixable gap found along the
//! way and deliberately NOT fixed here (scope creep beyond this
//! feature): a Magician/Necromancer summoned pet with its own unique
//! flavor name (bridged to its owner via `Ingest`'s pending_summons/
//! pet_owner map, confirmed real via "Greater Conjuration: Fire" ->
//! "Xasartik" in the reference log) never gets `Kind::Pet` from
//! `Entities::observe` at all -- that only recognizes the possessive
//! "<Owner>'s pet" suffix shape, a different pet-naming convention. Such
//! a pet reads Unproven forever unless/until GroupTracker happens to
//! reinforce it too.
//!
//! Neither channel ever touches `Kind` -- see `eqlp-app`'s
//! `Ingest::effective_kind`, which layers this on top of it for callers
//! that want "currently one of my allies" rather than "ever proven any
//! kind of ally".

use crate::fold_key;
use std::collections::HashMap;

pub type Millis = i64;

/// why: below this, weak (shared-target) evidence alone never counts --
/// see module doc's real-log measurement for why this is 5, not 2
pub const MIN_SESSIONS: u32 = 5;
/// why: 2h -- the real gap used to separate this log's own distinct play
/// sessions when picking MIN_SESSIONS above
pub const SESSION_GAP_MS: Millis = 2 * 60 * 60 * 1000;
/// why: how long since last reinforcement (either channel) before
/// "currently grouped" lapses -- generous against a real lull (loot,
/// travel, a longer fight elsewhere) but short enough a stale entry
/// doesn't linger across a whole play session
pub const GROUP_TTL_MS: Millis = 30 * 60 * 1000;

#[derive(Debug, Clone, Copy)]
struct Evidence {
    last_ms: Millis,
    sessions: u32,
    strong: bool,
}

impl Evidence {
    /// why: single source of truth for the gate -- currently_grouped and
    /// current_members must never drift apart on what "current" means
    fn is_current(&self, ts: Millis) -> bool {
        if ts - self.last_ms > GROUP_TTL_MS {
            return false;
        }
        self.strong || self.sessions >= MIN_SESSIONS
    }
}

/// why: monotonic-evidence keying (fold_key) mirrors Entities -- same
/// name, same identity, regardless of which struct is asked
#[derive(Debug, Default)]
pub struct GroupTracker {
    entries: HashMap<String, Evidence>,
}

impl GroupTracker {
    pub fn reinforce_weak(&mut self, name: &str, ts: Millis) {
        let key = fold_key(name);
        let e = self.entries.entry(key).or_insert(Evidence {
            last_ms: ts,
            sessions: 0,
            strong: false,
        });
        // why: sessions==0 catches the very first reinforcement (last_ms
        // was just seeded to ts above, so the gap check alone would miss it)
        if e.sessions == 0 || ts - e.last_ms > SESSION_GAP_MS {
            e.sessions += 1;
        }
        e.last_ms = e.last_ms.max(ts);
    }

    pub fn reinforce_strong(&mut self, name: &str, ts: Millis) {
        let key = fold_key(name);
        let e = self.entries.entry(key).or_insert(Evidence {
            last_ms: ts,
            sessions: 0,
            strong: false,
        });
        e.strong = true;
        e.last_ms = e.last_ms.max(ts);
    }

    /// why: raw (last_ms, sessions, strong) for diagnostics/debug UI --
    /// currently_grouped is the real answer, this is "why"
    pub fn evidence_for(&self, name: &str) -> Option<(Millis, u32, bool)> {
        self.entries
            .get(&fold_key(name))
            .map(|e| (e.last_ms, e.sessions, e.strong))
    }

    /// why: stale past GROUP_TTL_MS regardless of channel; the weak
    /// channel additionally needs the session gate, strong never does --
    /// see module doc for why the two channels earn that differently
    pub fn currently_grouped(&self, name: &str, ts: Millis) -> bool {
        self.entries
            .get(&fold_key(name))
            .is_some_and(|e| e.is_current(ts))
    }

    /// why: currently_grouped answers one name at a time; a debug/Game
    /// State dump needs the whole roster as of ts. Keys are fold_key'd
    /// (lowercase first char), not display casing -- callers resolve
    /// through Entities::display_name the same way `Ingest` does elsewhere.
    pub fn current_members(&self, ts: Millis) -> Vec<(String, u32, bool, Millis)> {
        self.entries
            .iter()
            .filter(|(_, e)| e.is_current(ts))
            .map(|(name, e)| (name.clone(), e.sessions, e.strong, e.last_ms))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_single_weak_reinforcement_is_not_enough() {
        let mut g = GroupTracker::default();
        g.reinforce_weak("Jabab", 1_000);
        assert!(!g.currently_grouped("Jabab", 1_000));
    }

    #[test]
    fn weak_evidence_within_the_same_session_never_crosses_the_gate() {
        let mut g = GroupTracker::default();
        // why: real shape of the reported false-positive -- dozens of
        // same-session hits, zero real recurrence
        for i in 0..80 {
            g.reinforce_weak("Jabab", 1_000 + i * 1_000);
        }
        assert!(!g.currently_grouped("Jabab", 80_000));
    }

    #[test]
    fn two_gap_separated_sessions_alone_still_does_not_cross_the_gate() {
        // why: real bug, measured against the reference log -- 2 was the
        // first threshold tried and still let a recurring farmable
        // charm-mob camp through (see module doc)
        let mut g = GroupTracker::default();
        let mut ts = 0;
        for _ in 0..2 {
            g.reinforce_weak("Dippinsauce", ts);
            ts += SESSION_GAP_MS + 1;
        }
        assert!(!g.currently_grouped("Dippinsauce", ts - SESSION_GAP_MS - 1));
    }

    #[test]
    fn weak_evidence_across_min_sessions_gap_separated_occasions_crosses_the_gate() {
        let mut g = GroupTracker::default();
        let mut ts = 0;
        for _ in 0..MIN_SESSIONS {
            g.reinforce_weak("Dippinsauce", ts);
            ts += SESSION_GAP_MS + 1;
        }
        let last = ts - SESSION_GAP_MS - 1;
        assert!(g.currently_grouped("Dippinsauce", last));
    }

    #[test]
    fn a_single_strong_reinforcement_is_enough() {
        let mut g = GroupTracker::default();
        g.reinforce_strong("Kaeus", 5_000);
        assert!(g.currently_grouped("Kaeus", 5_000));
    }

    #[test]
    fn confidence_lapses_after_the_ttl_with_no_reinforcement() {
        let mut g = GroupTracker::default();
        g.reinforce_strong("Kaeus", 0);
        assert!(g.currently_grouped("Kaeus", GROUP_TTL_MS));
        assert!(!g.currently_grouped("Kaeus", GROUP_TTL_MS + 1));
    }

    #[test]
    fn name_folding_matches_across_calls_like_entities_does() {
        let mut g = GroupTracker::default();
        g.reinforce_strong("kaeus", 0);
        assert!(g.currently_grouped("Kaeus", 0));
    }
}
