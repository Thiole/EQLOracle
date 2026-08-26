//! why: the overlay's Skill Tracker widget -- Spencer's ask: pick any
//! ability or spell to track (a real "track" action wherever one shows
//! up in the app -- Spellbook, Combat's ability rows, ...), flag when
//! each is estimated ready again, and whether the most recent attempt
//! landed. Not restricted to a curated list -- which ones actually show
//! in the overlay is the frontend's own `tracked_skills` preference,
//! populated by those track buttons; this module tracks every real
//! ability use unconditionally, cheap either way (bounded by how many
//! distinct abilities one character actually uses, not a fixed set).
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
//! Two real triggers feed this, picked to never double-count the same
//! real action twice:
//! - Melee abilities (Kick/Bash/Backstab/... and ordinary weapon swings
//!   alike): observed off Damage/Miss events, landed vs avoided is real.
//! - Spells: observed off cast.begin instead of any Damage event a
//!   damage spell might also produce -- a cast has no useful outcome
//!   yet at that instant, so `landed` is just `true` there (this
//!   module's own "last outcome" field isn't meaningful for spells,
//!   the target-effects section is where a resisted spell cast shows
//!   up for real).

use crate::ingest::Ingest;
use eqlp_source::Millis;
use serde::Serialize;

#[derive(Debug, Clone, Copy)]
pub struct SkillTrack {
    pub last_used_ms: Millis,
    pub last_landed: bool,
    /// why: None until a second real use gives an actual gap to learn from
    pub min_gap_ms: Option<i64>,
}

impl SkillTrack {
    fn first(ts: Millis, landed: bool) -> Self {
        SkillTrack {
            last_used_ms: ts,
            last_landed: landed,
            min_gap_ms: None,
        }
    }

    /// why: `gap > 0` guard -- the log's own 1-second resolution can give
    /// two real, distinct uses the same timestamp; a bogus 0ms "reuse
    /// timer" from that must never overwrite a real observed gap
    fn observe(&mut self, ts: Millis, landed: bool) {
        let gap = ts - self.last_used_ms;
        if gap > 0 {
            self.min_gap_ms = Some(self.min_gap_ms.map_or(gap, |m| m.min(gap)));
        }
        self.last_used_ms = ts;
        self.last_landed = landed;
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

#[derive(Debug, Clone, Serialize)]
pub struct SkillStatusDto {
    pub skill: String,
    pub last_outcome: &'static str,
    pub last_used_ms: Millis,
    /// why: None until a second real use gives an actual gap to learn from
    pub estimated_interval_ms: Option<i64>,
    /// why: None exactly when estimated_interval_ms is None -- nothing
    /// to compare "now" against yet
    pub ready: Option<bool>,
    /// why: 0 once ready, None when estimated_interval_ms is None
    pub remaining_ms: Option<i64>,
}

/// why: the overlay's own poll -- same polled-on-tick shape as
/// combat::live_meter/effects::status_effects. Every ability the player
/// has ever used at least once, regardless of which the user picked to
/// display -- see this module's own doc for why filtering which ones
/// actually show is the frontend's job, not this query's.
pub fn skill_status(ing: &Ingest) -> Vec<SkillStatusDto> {
    let now = ing.now_ms();
    ing.skills
        .iter()
        .map(|(skill, t)| {
            let ready_at = t.min_gap_ms.map(|g| t.last_used_ms + g);
            SkillStatusDto {
                skill: skill.clone(),
                last_outcome: if t.last_landed { "landed" } else { "avoided" },
                last_used_ms: t.last_used_ms,
                estimated_interval_ms: t.min_gap_ms,
                ready: ready_at.map(|r| now >= r),
                remaining_ms: ready_at.map(|r| (r - now).max(0)),
            }
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
        assert_eq!(t.min_gap_ms, None);
    }

    #[test]
    fn the_smallest_observed_gap_wins_even_if_seen_first() {
        let mut skills = HashMap::new();
        observe_skill_use(&mut skills, 0, "Kick", true);
        observe_skill_use(&mut skills, 2000, "Kick", false); // gap 2000
        observe_skill_use(&mut skills, 3500, "Kick", true); // gap 1500, smaller
        observe_skill_use(&mut skills, 8000, "Kick", true); // gap 4500, larger -- ignored
        let t = skills["Kick"];
        assert_eq!(t.min_gap_ms, Some(1500));
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
        assert_eq!(t.min_gap_ms, Some(2000));
    }

    /// why: real change this turn -- Spencer's ask generalized tracking
    /// off a curated 6-skill list to "anything with a track button",
    /// including plain weapon-swing verbs if the user picks one
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
        assert_eq!(s.estimated_interval_ms, Some(240_000));
    }
}
