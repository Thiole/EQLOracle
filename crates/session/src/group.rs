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
//! Three reinforcement channels, genuinely different confidence levels:
//! - `joined` (an explicit roster line -- "<Name> has joined the group",
//!   a group-chat message, an accepted invite): the game itself stated
//!   membership, no decay at all. Ends only by an explicit exit (`left`,
//!   `reset`) -- the join/leave lines are symmetric in the log, so a
//!   membership that started with a line ends with one (or with a
//!   whole-roster reset; see `reset`).
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

/// why: which channel is keeping a member current -- certainty descends
/// down the list, and the UI labels each differently
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    /// An explicit roster line named them. No decay.
    Joined,
    /// A corroborated Quick Buff group-cast landing, within GROUP_TTL_MS.
    Strong,
    /// Session-gated shared-target damage, within GROUP_TTL_MS.
    Weak,
}

impl Channel {
    pub fn name(self) -> &'static str {
        match self {
            Channel::Joined => "joined",
            Channel::Strong => "strong",
            Channel::Weak => "weak",
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Evidence {
    last_ms: Millis,
    sessions: u32,
    /// why: timestamp of the most recent strong reinforcement, not a
    /// bare bool -- real bug, found on review: a plain `bool` that only
    /// ever flips true and never resets makes one lifetime strong
    /// confirmation a permanent bypass of MIN_SESSIONS, revivable by a
    /// single later incidental weak hit (which alone wouldn't qualify)
    /// no matter how long ago the strong evidence itself actually
    /// happened. Strong evidence must decay past GROUP_TTL_MS same as
    /// everything else -- see `strong_currently`.
    strong_last_ms: Option<Millis>,
    /// why: an explicit roster line ("has joined the group") -- no TTL,
    /// the game states the fact; ends only via evicted_ms/reset_ms
    joined_ms: Option<Millis>,
    /// why: this one member's own explicit exit ("has left the group") --
    /// evidence at or before this instant no longer counts as current;
    /// session history survives, so a rejoin re-qualifies normally
    evicted_ms: Option<Millis>,
}

impl Evidence {
    /// why: single source of truth for the gate -- currently_grouped and
    /// current_members must never drift apart on what "current" means.
    /// `cutoff` is the later of this entry's own eviction and the
    /// tracker-wide reset; evidence at or after it counts. >= not > --
    /// log timestamps are whole seconds, and the real accept sequence
    /// ("You have joined" resetting the roster, the inviter added in the
    /// same second) must let that same-instant join survive its own reset.
    fn channel(&self, ts: Millis, reset_ms: Option<Millis>) -> Option<Channel> {
        let cutoff = self.evicted_ms.max(reset_ms);
        let fresh = |t: Option<Millis>| t.is_some_and(|t| Some(t) >= cutoff);
        if fresh(self.joined_ms) {
            return Some(Channel::Joined);
        }
        if ts - self.last_ms > GROUP_TTL_MS || !fresh(Some(self.last_ms)) {
            return None;
        }
        if self.strong_currently(ts) && fresh(self.strong_last_ms) {
            return Some(Channel::Strong);
        }
        (self.sessions >= MIN_SESSIONS).then_some(Channel::Weak)
    }

    /// why: split out of channel so the TTL check isn't duplicated
    fn strong_currently(&self, ts: Millis) -> bool {
        self.strong_last_ms.is_some_and(|s| ts - s <= GROUP_TTL_MS)
    }
}

/// why: monotonic-evidence keying (fold_key) mirrors Entities -- same
/// name, same identity, regardless of which struct is asked
#[derive(Debug, Default)]
pub struct GroupTracker {
    entries: HashMap<String, Evidence>,
    /// why: the whole roster's cutoff -- "You" left/were removed/joined a
    /// different group, or the log itself gapped a whole play session.
    /// Evidence at or before this instant no longer makes anyone current;
    /// session history survives (long-term recurrence is still real).
    reset_ms: Option<Millis>,
}

impl GroupTracker {
    fn entry(&mut self, name: &str, ts: Millis) -> &mut Evidence {
        self.entries.entry(fold_key(name)).or_insert(Evidence {
            last_ms: ts,
            sessions: 0,
            strong_last_ms: None,
            joined_ms: None,
            evicted_ms: None,
        })
    }

    pub fn reinforce_weak(&mut self, name: &str, ts: Millis) {
        let e = self.entry(name, ts);
        // why: sessions==0 catches the very first reinforcement (last_ms
        // was just seeded to ts above, so the gap check alone would miss it)
        if e.sessions == 0 || ts - e.last_ms > SESSION_GAP_MS {
            e.sessions += 1;
        }
        e.last_ms = e.last_ms.max(ts);
    }

    pub fn reinforce_strong(&mut self, name: &str, ts: Millis) {
        let e = self.entry(name, ts);
        e.strong_last_ms = Some(e.strong_last_ms.map_or(ts, |s| s.max(ts)));
        e.last_ms = e.last_ms.max(ts);
    }

    /// why: an explicit roster statement -- a join line, a group-chat
    /// message, an accepted invite. Definitive as of ts, no decay.
    pub fn joined(&mut self, name: &str, ts: Millis) {
        let e = self.entry(name, ts);
        e.joined_ms = Some(e.joined_ms.map_or(ts, |j| j.max(ts)));
        e.last_ms = e.last_ms.max(ts);
    }

    /// why: this one member's explicit exit line -- ends every channel's
    /// currency for them at ts without erasing session history (a
    /// linkdead groupmate who rejoins in two minutes shouldn't restart
    /// from zero gap-separated sessions). joined_ms cleared directly,
    /// not just out-cutoff'd: timestamps are whole seconds, so a
    /// same-second join+leave pair resolves by call order (log order),
    /// which the eviction comparison alone can't express.
    pub fn left(&mut self, name: &str, ts: Millis) {
        let e = self.entry(name, ts);
        e.evicted_ms = Some(e.evicted_ms.map_or(ts, |v| v.max(ts)));
        e.joined_ms = None;
    }

    /// why: the whole party is over for "You" -- removed/disbanded,
    /// joined a fresh group, or the log gapped past a real play session.
    /// One epoch, not per-entry mutation: O(1), and late re-evidence
    /// after the reset revives per channel rules exactly like `left`.
    pub fn reset(&mut self, ts: Millis) {
        self.reset_ms = Some(self.reset_ms.map_or(ts, |r| r.max(ts)));
    }

    /// why: raw (last_ms, sessions, strong_last_ms) for diagnostics/debug
    /// UI -- currently_grouped is the real answer, this is "why"
    pub fn evidence_for(&self, name: &str) -> Option<(Millis, u32, Option<Millis>)> {
        self.entries
            .get(&fold_key(name))
            .map(|e| (e.last_ms, e.sessions, e.strong_last_ms))
    }

    /// why: stale past GROUP_TTL_MS regardless of channel (explicit joins
    /// excepted -- the game stated the fact); the weak channel
    /// additionally needs the session gate, strong never does -- see
    /// module doc for why each channel earns that differently
    pub fn currently_grouped(&self, name: &str, ts: Millis) -> bool {
        self.entries
            .get(&fold_key(name))
            .is_some_and(|e| e.channel(ts, self.reset_ms).is_some())
    }

    /// why: currently_grouped answers one name at a time; a debug/Game
    /// State dump needs the whole roster as of ts. Keys are fold_key'd
    /// (lowercase first char), not display casing -- callers resolve
    /// through Entities::display_name the same way `Ingest` does elsewhere.
    /// The Channel is whichever evidence is *currently* keeping the entry
    /// current (as of `ts`), not the strongest it has ever been.
    pub fn current_members(&self, ts: Millis) -> Vec<(String, u32, Channel, Millis)> {
        self.entries
            .iter()
            .filter_map(|(name, e)| {
                e.channel(ts, self.reset_ms)
                    .map(|via| (name.clone(), e.sessions, via, e.last_ms))
            })
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

    /// why: real bug, found on review -- `strong` used to be a plain
    /// bool that only ever flipped true and never reset, so a single
    /// lifetime strong confirmation stayed a permanent MIN_SESSIONS
    /// bypass. Confirms a decayed strong confirmation stays decayed even
    /// once *some* reinforcement (a lone weak hit, nowhere near
    /// MIN_SESSIONS on its own) touches the entry again.
    #[test]
    fn a_decayed_strong_confirmation_does_not_revive_via_a_later_lone_weak_hit() {
        let mut g = GroupTracker::default();
        g.reinforce_strong("Kaeus", 0);
        let after_ttl = GROUP_TTL_MS + 1;
        assert!(!g.currently_grouped("Kaeus", after_ttl));

        g.reinforce_weak("Kaeus", after_ttl + 1);
        assert!(
            !g.currently_grouped("Kaeus", after_ttl + 1),
            "one incidental shared-target hit must not resurrect a long-decayed strong confirmation"
        );
    }

    /// why: companion to the above -- a genuinely *fresh* strong
    /// reinforcement (not leftover state) does renew normally
    #[test]
    fn a_fresh_strong_reinforcement_renews_after_the_first_decayed() {
        let mut g = GroupTracker::default();
        g.reinforce_strong("Kaeus", 0);
        let after_ttl = GROUP_TTL_MS + 1;
        assert!(!g.currently_grouped("Kaeus", after_ttl));

        g.reinforce_strong("Kaeus", after_ttl);
        assert!(g.currently_grouped("Kaeus", after_ttl));
    }

    #[test]
    fn name_folding_matches_across_calls_like_entities_does() {
        let mut g = GroupTracker::default();
        g.reinforce_strong("kaeus", 0);
        assert!(g.currently_grouped("Kaeus", 0));
    }

    #[test]
    fn an_explicit_join_is_definitive_and_does_not_decay() {
        let mut g = GroupTracker::default();
        g.joined("Dippinsauce", 0);
        assert!(g.currently_grouped("Dippinsauce", 0));
        // why: hours later, no reinforcement at all -- still a member
        // until an explicit exit; join/leave lines are symmetric
        assert!(g.currently_grouped("Dippinsauce", 5 * 60 * 60 * 1000));
    }

    #[test]
    fn an_explicit_leave_ends_membership_for_that_member_only() {
        let mut g = GroupTracker::default();
        g.joined("Dippinsauce", 0);
        g.joined("Bravesirrobin", 0);
        g.left("Dippinsauce", 10_000);
        assert!(!g.currently_grouped("Dippinsauce", 10_000));
        assert!(g.currently_grouped("Bravesirrobin", 10_000));
    }

    #[test]
    fn a_rejoin_after_leaving_revives_membership() {
        let mut g = GroupTracker::default();
        g.joined("Dippinsauce", 0);
        g.left("Dippinsauce", 10_000);
        g.joined("Dippinsauce", 20_000);
        assert!(g.currently_grouped("Dippinsauce", 20_000));
    }

    #[test]
    fn reset_clears_every_channel_at_once() {
        let mut g = GroupTracker::default();
        g.joined("Dippinsauce", 0);
        g.reinforce_strong("Kaeus", 0);
        let mut ts = 0;
        for _ in 0..MIN_SESSIONS {
            g.reinforce_weak("Wynvern", ts);
            ts += SESSION_GAP_MS + 1;
        }
        let last_weak = ts - SESSION_GAP_MS - 1;
        assert!(g.currently_grouped("Wynvern", last_weak));

        let disband = last_weak + 1;
        g.reset(disband);
        assert!(!g.currently_grouped("Dippinsauce", disband + 1));
        assert!(!g.currently_grouped("Kaeus", disband + 1));
        assert!(!g.currently_grouped("Wynvern", disband + 1));
    }

    /// why: session history deliberately survives a reset -- one fresh
    /// shared-target hit after a disband re-qualifies a longtime
    /// groupmate (their recurrence is still real), while a stranger
    /// still needs MIN_SESSIONS from scratch
    #[test]
    fn fresh_weak_evidence_after_a_reset_requalifies_a_longtime_groupmate() {
        let mut g = GroupTracker::default();
        let mut ts = 0;
        for _ in 0..MIN_SESSIONS {
            g.reinforce_weak("Wynvern", ts);
            ts += SESSION_GAP_MS + 1;
        }
        g.reset(ts);
        assert!(!g.currently_grouped("Wynvern", ts + 1));
        g.reinforce_weak("Wynvern", ts + 2);
        assert!(g.currently_grouped("Wynvern", ts + 2));
    }

    #[test]
    fn an_explicit_leave_beats_older_strong_evidence() {
        let mut g = GroupTracker::default();
        g.reinforce_strong("Kaeus", 1_000);
        g.left("Kaeus", 2_000);
        assert!(!g.currently_grouped("Kaeus", 2_000));
        // why: fresh strong evidence after the exit revives normally
        g.reinforce_strong("Kaeus", 3_000);
        assert!(g.currently_grouped("Kaeus", 3_000));
    }

    #[test]
    fn current_members_reports_the_channel_keeping_each_entry_current() {
        let mut g = GroupTracker::default();
        g.joined("Dippinsauce", 0);
        g.reinforce_strong("Kaeus", 0);
        let members = g.current_members(0);
        let via = |name: &str| {
            members
                .iter()
                .find(|(n, ..)| n == &fold_key(name))
                .map(|&(_, _, via, _)| via)
        };
        assert_eq!(via("Dippinsauce"), Some(Channel::Joined));
        assert_eq!(via("Kaeus"), Some(Channel::Strong));
    }
}
