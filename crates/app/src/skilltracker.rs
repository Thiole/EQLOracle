//! why: the overlay's Skill Tracker widget -- pick any ability or spell
//! to track (a "track" button wherever one shows up -- Spellbook,
//! Combat's ability rows), flag when it's estimated ready again, and
//! whether the last attempt landed. Not a curated list -- which ones
//! show in the overlay is the frontend's `tracked_skills` preference;
//! this module tracks every real ability use unconditionally (bounded
//! by how many distinct abilities one character actually uses).
//!
//! No hardcoded reuse-timer table -- this is a custom server, official
//! wiki numbers (already level/AA-modified anyway) aren't trustworthy
//! here. Instead the reuse interval is estimated empirically: the
//! smallest real gap ever observed between two of the player's own
//! uses of that ability. A skill's true reuse timer is a hard floor
//! nothing can beat, so the smallest observed gap is the best available
//! lower bound, and only ever improves (shrinks toward truth) as more
//! real uses are seen -- same "trust the log over a wiki number" stance
//! this app takes everywhere else (spell resist types, class detection, ...).
//!
//! Two real triggers feed the *attempt* signal, picked to never
//! double-count the same real action twice:
//! - Melee abilities (Kick/Bash/Backstab/... and ordinary weapon swings
//!   alike): observed off Damage/Miss events, landed vs avoided is real.
//! - Spells: observed off cast.begin instead of any Damage event a
//!   damage spell might also produce -- a cast has no useful outcome
//!   yet at that instant, so `landed` is just `true` there (this
//!   module's own "last outcome" field isn't meaningful for spells,
//!   the target-effects section is where a resisted spell cast shows
//!   up for real).
//!
//! Real mechanic on this server: cooldown and recovery timer, whichever
//! is longer. This used to estimate readiness off attempt-to-attempt gap
//! alone (`reuse`); a *second* signal, `recovery`, is measured from a
//! confirmed LANDING (not the attempt) to the next attempt -- fed off
//! the same `CastResolver::confirm_landed` timestamp ingest.rs already
//! resolves, no new parsing. Reuse alone isn't trustworthy: it's fed by
//! every attempt including resists/misses, which don't carry the same
//! lockout a real landing does, so a fast resisted-then-recast sample
//! can make reuse look shorter than the real minimum. Final readiness
//! is whichever of the two clears later -- never the optimistic one.

use crate::ingest::Ingest;
use eqlp_source::Millis;
use serde::Serialize;

#[derive(Debug, Clone, Copy)]
pub struct SkillTrack {
    pub last_used_ms: Millis,
    pub last_landed: bool,
    /// why: None until a second real attempt gives an actual gap to learn from
    pub reuse_gap_ms: Option<i64>,
    /// why: None until a real landing has ever confirmed for this skill
    /// at all -- not every attempt lands, and some abilities (a pure
    /// melee swing) never resolve through CastResolver in the first
    /// place, so this stays None forever for those, same as recovery_gap_ms
    pub last_landed_ms: Option<Millis>,
    /// why: None until a landing AND a later attempt both exist
    pub recovery_gap_ms: Option<i64>,
}

impl SkillTrack {
    fn first(ts: Millis, landed: bool) -> Self {
        SkillTrack {
            last_used_ms: ts,
            last_landed: landed,
            reuse_gap_ms: None,
            last_landed_ms: None,
            recovery_gap_ms: None,
        }
    }

    /// why: `gap > 0` guard -- the log's own 1-second resolution can give
    /// two real, distinct uses the same timestamp; a bogus 0ms "reuse
    /// timer" from that must never overwrite a real observed gap
    fn observe(&mut self, ts: Millis, landed: bool) {
        let gap = ts - self.last_used_ms;
        if gap > 0 {
            self.reuse_gap_ms = Some(self.reuse_gap_ms.map_or(gap, |m| m.min(gap)));
        }
        if let Some(landed_ms) = self.last_landed_ms {
            let rgap = ts - landed_ms;
            if rgap > 0 {
                self.recovery_gap_ms = Some(self.recovery_gap_ms.map_or(rgap, |m| m.min(rgap)));
            }
        }
        self.last_used_ms = ts;
        self.last_landed = landed;
    }

    /// why: a confirmed landing, separate from (and usually a little
    /// after) the attempt that caused it -- see this module's own doc
    fn observe_landing(&mut self, ts: Millis) {
        self.last_landed_ms = Some(ts);
    }

    /// why: reuse and recovery are measured from different anchors and
    /// can't be combined into one "anchor + interval" pair (SkillStatusDto
    /// exposes the resolved absolute deadline directly instead, see its
    /// own doc) -- whichever of the two clears later is the real answer,
    /// see this module's own top-level doc for why neither alone is safe
    fn ready_at(&self) -> Option<Millis> {
        let reuse = self.reuse_gap_ms.map(|g| self.last_used_ms + g);
        let recovery = match (self.last_landed_ms, self.recovery_gap_ms) {
            (Some(landed_ms), Some(g)) => Some(landed_ms + g),
            _ => None,
        };
        match (reuse, recovery) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        }
    }
}

/// why: called from record_damage (melee only, landed=true),
/// record_avoided (landed=false), and Action::Cast's own handler
/// (spells, landed=true -- see this module's own doc for why melee and
/// spells use different triggers)
pub fn observe_skill_use(
    skills: &mut std::collections::HashMap<String, SkillTrack>,
    ts: Millis,
    ability: &str,
    landed: bool,
) {
    match skills.get_mut(ability) {
        Some(t) => t.observe(ts, landed),
        None => {
            skills.insert(ability.to_string(), SkillTrack::first(ts, landed));
        }
    }
}

/// why: called from flush_cast_resolutions when a cast resolves Landed
/// for "You" -- confirm_landed's own real timestamp (whichever of
/// Damage/Heal/a spelltext-matched landing line actually confirmed it),
/// a separate, later signal than cast.begin's own attempt timestamp.
/// A landing with no tracked attempt behind it (name mismatch, or
/// nothing else ever tracked it) is a safe no-op, not a fresh entry --
/// there's nothing meaningful to time a recovery gap against yet.
pub fn observe_skill_landed(
    skills: &mut std::collections::HashMap<String, SkillTrack>,
    ts: Millis,
    ability: &str,
) {
    if let Some(t) = skills.get_mut(ability) {
        t.observe_landing(ts);
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SkillStatusDto {
    pub skill: String,
    pub last_outcome: &'static str,
    pub last_used_ms: Millis,
    /// why: already resolved as max(reuse, recovery) -- see SkillTrack::
    /// ready_at's own doc for why the two can't be exposed as a single
    /// "anchor + interval" pair. A real absolute deadline (same
    /// countdown-from-a-deadline shape targeteffects.rs's own
    /// ready_at_ms uses), not a relative duration that would go stale
    /// between polls. None only when there's no data to estimate from
    /// at all yet (a single attempt, nothing landed).
    pub ready_at_ms: Option<Millis>,
    /// why: the raw learned interval behind ready_at_ms, not just the
    /// resolved deadline -- the Skill Data tab wants the actual timer
    /// LENGTH, meaningful long after last_used_ms is stale. Smallest gap
    /// ever observed between two real attempts -- see this module's
    /// top-level doc for why that's the only trustworthy source here (no
    /// hardcoded wiki table); already reflects AA/haste/gear since it's
    /// measured off the player's own real casts.
    pub reuse_gap_ms: Option<i64>,
    /// why: the recovery-anchor counterpart to reuse_gap_ms -- see
    /// SkillTrack::ready_at's own doc for why landing-to-next-attempt is
    /// tracked completely separately from attempt-to-attempt
    pub recovery_gap_ms: Option<i64>,
}

/// why: the overlay's own poll -- same polled-on-tick shape as
/// combat::live_meter/effects::status_effects. Every ability the player
/// has ever used at least once, regardless of which the user picked to
/// display -- see this module's own doc for why filtering which ones
/// actually show is the frontend's job, not this query's.
pub fn skill_status(ing: &Ingest) -> Vec<SkillStatusDto> {
    // why: `skills` is a HashMap, so an unsorted walk handed the UI a
    // different order every launch -- the skill list visibly reshuffled
    let mut by_name: Vec<_> = ing.skills.iter().collect();
    by_name.sort_by(|a, b| a.0.cmp(b.0));
    by_name
        .into_iter()
        .map(|(skill, t)| SkillStatusDto {
            skill: skill.clone(),
            last_outcome: if t.last_landed { "landed" } else { "avoided" },
            last_used_ms: t.last_used_ms,
            ready_at_ms: t.ready_at(),
            reuse_gap_ms: t.reuse_gap_ms,
            recovery_gap_ms: t.recovery_gap_ms,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn a_single_use_has_no_estimate_yet() {
        let mut skills = HashMap::new();
        observe_skill_use(&mut skills, 1000, "Kick", true);
        let t = skills["Kick"];
        assert_eq!(t.reuse_gap_ms, None);
    }

    #[test]
    fn the_smallest_observed_gap_wins_even_if_seen_first() {
        let mut skills = HashMap::new();
        observe_skill_use(&mut skills, 0, "Kick", true);
        observe_skill_use(&mut skills, 2000, "Kick", false); // gap 2000
        observe_skill_use(&mut skills, 3500, "Kick", true); // gap 1500, smaller
        observe_skill_use(&mut skills, 8000, "Kick", true); // gap 4500, larger -- ignored
        let t = skills["Kick"];
        assert_eq!(t.reuse_gap_ms, Some(1500));
        assert_eq!(t.last_used_ms, 8000);
        assert!(t.last_landed);
    }

    #[test]
    fn a_same_timestamp_repeat_never_produces_a_zero_gap() {
        let mut skills = HashMap::new();
        observe_skill_use(&mut skills, 1000, "Kick", true);
        observe_skill_use(&mut skills, 1000, "Kick", false); // same log second
        observe_skill_use(&mut skills, 3000, "Kick", true); // real gap: 2000
        let t = skills["Kick"];
        assert_eq!(t.reuse_gap_ms, Some(2000));
    }

    /// why: recovery_gap_ms tracks its own independent minimum, only
    /// ever updated at an attempt that has a landing already behind it --
    /// a landing-less (resisted) fast recast tightens reuse_gap_ms
    /// without ever touching recovery_gap_ms
    #[test]
    fn recovery_gap_only_updates_at_an_attempt_with_a_landing_behind_it() {
        let mut skills = HashMap::new();
        observe_skill_use(&mut skills, 0, "Wandering Mind", true);
        observe_skill_landed(&mut skills, 3, "Wandering Mind");
        observe_skill_use(&mut skills, 90, "Wandering Mind", true); // reuse 90, recovery 90-3=87
                                                                    // no landing this cycle (resisted) -- a fast retry right after
        observe_skill_use(&mut skills, 95, "Wandering Mind", true); // reuse min(90,5)=5, recovery untouched
        let t = skills["Wandering Mind"];
        assert_eq!(t.reuse_gap_ms, Some(5), "the fluke fast resisted retry");
        assert_eq!(
            t.recovery_gap_ms,
            Some(87),
            "no landing since 3, so unaffected by the fast retry"
        );
    }

    /// why: cooldown vs. recovery timer, whichever is longer. Tested
    /// against the resolved struct, not a call sequence -- a landing can
    /// confirm after a later attempt already fired (slow landing line
    /// after a quick resisted retry), so last_landed_ms > last_used_ms
    /// is a real state -- see the doc on SkillTrack::ready_at.
    #[test]
    fn ready_at_uses_whichever_of_reuse_or_recovery_clears_later() {
        let recovery_wins = SkillTrack {
            last_used_ms: 100,
            last_landed: true,
            reuse_gap_ms: Some(20),    // reuse ready at 120
            last_landed_ms: Some(150), // recovery ready at 150+90=240
            recovery_gap_ms: Some(90),
        };
        assert_eq!(recovery_wins.ready_at(), Some(240));

        let reuse_wins = SkillTrack {
            last_used_ms: 200,
            last_landed: true,
            reuse_gap_ms: Some(90), // reuse ready at 290
            last_landed_ms: Some(150),
            recovery_gap_ms: Some(20), // recovery ready at 170
        };
        assert_eq!(reuse_wins.ready_at(), Some(290));
    }

    /// why: a landing with no attempt behind it (name mismatch, or an
    /// entry that was never tracked as an attempt at all) is a no-op,
    /// not a fresh entry -- nothing meaningful to time a recovery
    /// gap against
    #[test]
    fn a_landing_with_no_tracked_attempt_is_a_safe_no_op() {
        let mut skills = HashMap::new();
        observe_skill_landed(&mut skills, 100, "Nothing Tracked");
        assert!(!skills.contains_key("Nothing Tracked"));
    }

    /// why: tracking generalized off a curated 6-skill list to "anything
    /// with a track button", including plain weapon-swing verbs
    #[test]
    fn any_ability_can_be_tracked_not_just_a_curated_list() {
        let mut skills = HashMap::new();
        observe_skill_use(&mut skills, 1000, "Slash", true);
        assert!(skills.contains_key("Slash"));
    }

    /// why: real change this turn -- a non-damage spell (no Damage event
    /// at all) must still be trackable, observed off cast.begin instead
    #[test]
    fn a_real_spell_cast_is_tracked_off_cast_begin_not_just_melee() {
        use crate::ingest::{backfill_lines, Ingest};
        use crate::parser::build_engine;
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Tue Jul 28 15:01:00 2026] You begin casting Spirit of Wolf.",
            b"[Tue Jul 28 15:05:00 2026] You begin casting Spirit of Wolf.",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);
        let statuses = skill_status(&ing);
        let s = statuses
            .iter()
            .find(|s| s.skill == "Spirit of Wolf")
            .expect("a self-buff with no damage event should still be tracked");
        // why: no landing confirmed in this minimal scenario (no Damage/
        // Heal/spelltext-matched line), so this is reuse alone
        assert_eq!(s.ready_at_ms.map(|r| r - s.last_used_ms), Some(240_000));
    }

    /// why: proves the real wiring end-to-end -- ingest.rs's own
    /// flush_cast_resolutions really does call observe_skill_landed off
    /// a genuine CastResolver::confirm_landed (here, the real Damage
    /// event a DoT tick produces), not just the pure combinator logic
    /// the unit tests above exercise directly
    #[test]
    fn a_real_landing_confirmed_through_ingest_feeds_the_recovery_clock() {
        use crate::ingest::{backfill_lines, Ingest};
        use crate::parser::build_engine;
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Tue Jul 28 15:01:00 2026] You begin casting Ignite Bones.",
            b"[Tue Jul 28 15:01:03 2026] You hit a rat for 3 points of magic damage by Ignite Bones.",
            b"[Tue Jul 28 15:02:00 2026] You begin casting Ignite Bones.",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);
        // why: asserted straight off the tracked entry, not skill_status's
        // own resolved ready_at_ms -- with only one sample each, reuse and
        // recovery mathematically land on the same final instant either
        // way (see the unit test above for a real divergence), so a DTO-
        // level assertion here couldn't actually tell "landing wired
        // correctly" apart from "landing never fired at all"
        let t = ing
            .skills
            .get("Ignite Bones")
            .expect("tracked off cast.begin");
        assert_eq!(t.reuse_gap_ms, Some(60_000), "15:02:00 - 15:01:00");
        assert_eq!(
            t.recovery_gap_ms,
            Some(57_000),
            "15:02:00 - 15:01:03, the real Damage-confirmed landing"
        );
    }

    /// why: real bug, caught live -- a debuff re-applied onto a target
    /// that already has it never produces a Damage event or the
    /// generic flavor-text landing at all, only "Your X spell on Y has
    /// been overwritten." (see ingest::Action::SpellOverwritten's own
    /// doc). That real landing must feed the recovery clock too, same
    /// as any other confirmed landing.
    #[test]
    fn an_overwritten_confirmation_also_feeds_the_recovery_clock() {
        use crate::ingest::{backfill_lines, Ingest};
        use crate::parser::build_engine;
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Tue Jul 28 15:01:00 2026] You begin casting Shiftless Deeds IV.",
            b"[Tue Jul 28 15:01:03 2026] Your Shiftless Deeds spell on a rat has been overwritten.",
            b"[Tue Jul 28 15:02:00 2026] You begin casting Shiftless Deeds IV.",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);
        let t = ing
            .skills
            .get("Shiftless Deeds")
            .expect("tracked off cast.begin");
        assert_eq!(t.reuse_gap_ms, Some(60_000), "15:02:00 - 15:01:00");
        assert_eq!(
            t.recovery_gap_ms,
            Some(57_000),
            "15:02:00 - 15:01:03, the overwritten-confirmed landing"
        );
    }
}
