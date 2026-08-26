//! why: the overlay's Skill Tracker widget -- Spencer's ask: track a
//! chosen set of combat skills, flag when each is estimated ready again,
//! and whether the most recent attempt landed. Scoped to the real
//! discrete special attacks (own reuse timer, not just a per-weapon swing
//! verb) -- Kick/Bash/Backstab/Frenzy/Smite/Reave all already flow
//! through the normal Damage/Miss pipeline as their own named ability
//! (see `ingest::canonical_melee_ability`), unlike Slash/Crush/Pierce/
//! Hit/Cleave/Punch/... which are just which verb a given weapon type
//! uses for an ordinary swing, not a specific skill with its own timer.
//!
//! No hardcoded reuse-timer table -- this is a custom server, official
//! wiki numbers (already level/AA-modified anyway) aren't trustworthy
//! here. Instead the reuse interval is estimated empirically: the
//! smallest real gap ever observed between two of the player's own
//! uses of that skill. A skill's true reuse timer is a hard floor nothing
//! can beat, so the smallest observed gap is the best available lower
//! bound, and only ever improves (shrinks toward truth) as more real
//! uses are seen -- same "trust the log over a wiki number" stance this
//! app takes everywhere else (spell resist types, class detection, ...).

use crate::ingest::Ingest;
use eqlp_source::Millis;
use serde::Serialize;

/// why: the real discrete special attacks with their own reuse timer --
/// see this module's own doc for why ordinary swing verbs don't belong
/// here. Selectable in Settings; skill_status reports whichever of these
/// the player has actually used at least once, regardless of selection --
/// cheap either way, the picker only controls what the overlay *shows*.
pub const TRACKED_SKILLS: &[&str] = &["Kick", "Bash", "Backstab", "Frenzy", "Smite", "Reave"];

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

/// why: called from record_damage (landed=true) and record_avoided
/// (landed=false) -- both already resolve the real actor name and
/// canonical ability name before this, so no re-parsing here
pub fn observe_skill_use(
    skills: &mut std::collections::HashMap<String, SkillTrack>,
    ts: Millis,
    ability: &str,
    landed: bool,
) {
    if !TRACKED_SKILLS.contains(&ability) {
        return;
    }
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
/// combat::live_meter/effects::status_effects. Every skill the player
/// has ever used at least once, regardless of which the user picked to
/// display -- see TRACKED_SKILLS' own doc for why filtering is the
/// frontend's job, not this query's.
pub fn skill_status(ing: &Ingest) -> Vec<SkillStatusDto> {
    let now = ing.now_ms();
    TRACKED_SKILLS
        .iter()
        .filter_map(|&skill| {
            let t = ing.skills.get(skill)?;
            let ready_at = t.min_gap_ms.map(|g| t.last_used_ms + g);
            Some(SkillStatusDto {
                skill: skill.to_string(),
                last_outcome: if t.last_landed { "landed" } else { "avoided" },
                last_used_ms: t.last_used_ms,
                estimated_interval_ms: t.min_gap_ms,
                ready: ready_at.map(|r| now >= r),
                remaining_ms: ready_at.map(|r| (r - now).max(0)),
            })
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

    #[test]
    fn an_untracked_ability_is_ignored() {
        let mut skills = HashMap::new();
        observe_skill_use(&mut skills, 1000, "Slash", true);
        assert!(skills.is_empty());
    }
}
