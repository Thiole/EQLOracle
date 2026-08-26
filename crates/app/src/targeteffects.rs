//! why: Skill Tracker's target-effects section -- Spencer's own ask:
//! "tracking should be done per target, so dots can be easily tracked
//! per target ... a target (ex: Lord Nagafen) that shows the icons for
//! tracked spell effects that were/tried on him, like slow with a
//! timer." Scoped to the player's own casts against the current fight's
//! target -- reuses combat::current_encounter (the same "most recently
//! ACTIVE real encounter" resolution live_meter already got right, not
//! just most recently opened).
//!
//! Two real signals feed this, both already-existing infrastructure, no
//! new parsing:
//! - DoT ticks: real Damage events, actor "You", tag::SPELL, targeted at
//!   the current mob -- the same stream combat.rs already reads for
//!   everything else.
//! - Everything else (debuff landings, resisted attempts): `Ingest::effects`,
//!   the per-entity ping history `attribute_effect`/`record_effect_ping`
//!   already builds for RecentEffectDto -- unbounded, already real,
//!   already attributes source+skill best-effort. A resisted cast now
//!   pushes here too (see ingest.rs's own CastResisted handling) so a
//!   failed attempt shows up right alongside a landed one.
//!
//! Duration comes from spelleffect::effects_for's own wiki-scraped
//! SpellDuration, by real spell name -- same data source
//! spelleffect.rs already ships to the frontend elsewhere. No wear-off
//! confirmation exists for most of these (Effects' own doc: "recency,
//! not a live still active claim"), so a timer reaching zero doesn't
//! mean gone for certain -- the frontend flashes it instead of dropping
//! it, until the target itself clears.

use crate::combat;
use crate::ingest::Ingest;
use crate::{spelldata, spelleffect};
use eqlp_session::{Allegiance, State};
use eqlp_source::Millis;
use eqlp_store::{tag, EventKind};
use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize)]
pub struct TargetEffectDto {
    pub spell: String,
    /// why: the wiki scrape's own icon filename (packs/spells.json) --
    /// real assets are bundled at ui/public/planner/icons, same ones
    /// SpellbookBuilder already renders; None for an unrecognized name
    pub icon: Option<String>,
    /// why: false when the most recent real observation was a resisted
    /// cast, not a landing -- flashed at 0:00 client-side
    pub landed: bool,
    pub since_ms: Millis,
    /// why: None for a failed cast (nothing landed to time) or a landed
    /// effect with no known real duration (nothing shown without one --
    /// see target_effects' own filter)
    pub duration_ms: Option<i64>,
    pub ready_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct TargetEffectsDto {
    /// why: None when there's no live enemy target to report against --
    /// see target_effects' own doc for the real clear conditions
    pub target: Option<String>,
    pub effects: Vec<TargetEffectDto>,
}

/// why: the wiki-scraped duration, filtered to what's actually worth a
/// countdown -- a permanent buff or an instant nuke isn't a "timed
/// effect" in the sense this widget means
fn duration_ms_for(spell: &str) -> Option<i64> {
    let s = spelldata::spell_by_name(spell)?;
    let d = spelleffect::effects_for(s).duration;
    if d.is_permanent || d.is_instant {
        return None;
    }
    d.max_secs.map(|secs| (secs * 1000.0).round() as i64)
}

/// why: the overlay's own poll -- same polled-on-tick shape as
/// combat::live_meter/effects::status_effects
pub fn target_effects(ing: &Ingest) -> TargetEffectsDto {
    let now = ing.now_ms();
    let Some(enc) = combat::current_encounter(ing) else {
        return TargetEffectsDto::default();
    };
    // why: Spencer's own ask -- clear the whole panel once the fight is
    // over, not linger showing stale effects against a mob no longer
    // being fought
    if !enc.is_open() {
        return TargetEffectsDto::default();
    }
    let target_sym = enc.target;
    let target_name = ing.store.name(target_sym).to_string();
    let kind = ing.encounters.entities.kind(&target_name);
    let state = ing
        .timeline
        .state_at(target_sym.0, now)
        .map(|(s, _)| s)
        .unwrap_or(State::Engaged);
    if !Allegiance::of(kind, state).is_enemy() {
        // why: charmed by a teammate (or otherwise no longer an enemy) --
        // Spencer's other named clear condition
        return TargetEffectsDto::default();
    }

    // why: Spencer's own ask -- "only show the highest for a skill
    // line". A DoT/debuff line can have several real ranks (the same
    // per-character rank system Spellbook's own toRoman/MAX_RANK picker
    // already deals with, see ingest::split_cast_rank's own doc); once
    // you've got the higher one, a stale lower-rank observation isn't
    // worth its own badge. Grouped by the line's own base name -- higher
    // rank always wins, recency only breaks a tie within the same rank
    // (or when neither side has a resolvable rank at all).
    //
    // Real bug, caught live against Spencer's own log ("Wandering
    // Mind"): a resisted cast keeps its rank suffix verbatim in the log
    // text ("resisted your Wandering Mind VI!"), but a LANDED cast is
    // only ever attributed through recent_casts, which is already
    // base_spell_name-stripped before it gets here -- so a landed
    // observation can never carry a resolvable rank, even when the real
    // cast was rank VI. Under rank-only comparison that landed
    // observation's `None` read as "known lower rank" and could never
    // beat an earlier resisted `Some(6)`, so a later real landing was
    // silently ignored and the panel stayed stuck on a stale resist
    // forever. Landed status is checked first now: a fresh landing is
    // real confirmed state and always supersedes an older failure
    // regardless of rank; a later failed *re*-attempt never erases a
    // landing that already happened. Rank only decides ties between two
    // observations of the same landed-ness.
    struct LineObs {
        full_name: String,
        rank: Option<u8>,
        ts: Millis,
        landed: bool,
    }
    let mut latest: HashMap<String, LineObs> = HashMap::new();
    let mut note = |full_name: String, ts: Millis, landed: bool| {
        let (base, rank) = crate::ingest::split_cast_rank(&full_name);
        let base = base.to_string();
        let candidate = LineObs {
            full_name,
            rank,
            ts,
            landed,
        };
        match latest.get_mut(&base) {
            Some(existing) => {
                let better = match (candidate.landed, existing.landed) {
                    (true, false) => true,
                    (false, true) => false,
                    _ => match (candidate.rank, existing.rank) {
                        (Some(r), Some(er)) => r > er || (r == er && candidate.ts >= existing.ts),
                        (Some(_), None) => true,
                        (None, Some(_)) => false,
                        (None, None) => candidate.ts >= existing.ts,
                    },
                };
                if better {
                    *existing = candidate;
                }
            }
            None => {
                latest.insert(base, candidate);
            }
        }
    };

    if let Some(you_sym) = ing.store.names.get("You") {
        for i in enc.range() {
            if ing.store.kind[i] != EventKind::Damage
                || ing.store.actor[i] != you_sym
                || ing.store.target[i] != target_sym
            {
                continue;
            }
            let ab = ing.store.ability[i];
            if ing.store.abilities.tags(ab) & tag::SPELL == 0 {
                continue;
            }
            note(
                ing.store.ability_name(ab).to_string(),
                ing.store.ts[i],
                true,
            );
        }
    }
    for p in ing.effects.all(target_sym.0) {
        let (Some(skill), Some(source)) = (&p.skill, &p.source) else {
            continue;
        };
        if !source.eq_ignore_ascii_case("you") {
            continue;
        }
        note(skill.clone(), p.ts, p.landed);
    }

    let mut effects: Vec<TargetEffectDto> = latest
        .into_values()
        .filter_map(|obs| {
            let LineObs {
                full_name: spell,
                ts: since_ms,
                landed,
                ..
            } = obs;
            let duration_ms = if landed {
                duration_ms_for(&spell)
            } else {
                None
            };
            if landed && duration_ms.is_none() {
                return None; // landed but no known duration -- nothing to time
            }
            let ready_at_ms = duration_ms.map(|d| since_ms + d);
            let icon = spelldata::spell_by_name(&spell).and_then(|s| s.icon.clone());
            Some(TargetEffectDto {
                spell,
                icon,
                landed,
                since_ms,
                duration_ms,
                ready_at_ms,
            })
        })
        .collect();
    effects.sort_by_key(|e| std::cmp::Reverse(e.since_ms));

    TargetEffectsDto {
        target: Some(target_name),
        effects,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::backfill_lines;
    use crate::parser::build_engine;

    fn run(lines: &[&str]) -> Ingest {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let bytes: Vec<&[u8]> = lines.iter().map(|l| l.as_bytes()).collect();
        backfill_lines(&mut ing, &engine, &bytes, 1);
        ing.tick(ing.now_ms());
        ing
    }

    /// why: encounter closure (confirmed-kill included) needs real
    /// elapsed idle time to finalize via Ingest::tick's own expire()
    /// pass, not just the kill line itself -- same real gap combat.rs's
    /// own ingest_from test helper works around, though its own filler
    /// is a damage hit, which here would just open a second, brand new
    /// current_encounter (defeating the point of testing that the FIRST
    /// one cleared) -- a harmless chat line advances the log clock the
    /// same way without opening any encounter of its own.
    fn run_to_closure(lines: &[&str]) -> Ingest {
        let mut all: Vec<&str> = lines.to_vec();
        all.push("[Tue Jul 28 15:06:00 2026] You tell your party, 'ready'");
        run(&all)
    }

    #[test]
    fn no_encounter_yet_reports_no_target() {
        let ing = run(&[]);
        let dto = target_effects(&ing);
        assert_eq!(dto.target, None);
        assert!(dto.effects.is_empty());
    }

    /// why: real spell (SK/Necro DoT), real "3 ticks" duration -- proves
    /// the whole DoT-tick pathway against real spell data, not a synthetic name
    #[test]
    fn a_real_dot_ticking_on_the_target_shows_up_with_a_real_duration() {
        let ing = run(&[
            "[Tue Jul 28 15:01:00 2026] You hit a rat for 3 points of magic damage by Ignite Bones.",
            "[Tue Jul 28 15:01:06 2026] You hit a rat for 3 points of magic damage by Ignite Bones.",
        ]);
        let dto = target_effects(&ing);
        assert_eq!(dto.target.as_deref(), Some("a rat"));
        let e = dto
            .effects
            .iter()
            .find(|e| e.spell == "Ignite Bones")
            .expect("Ignite Bones should be tracked");
        assert!(e.landed);
        assert_eq!(e.duration_ms, Some(18_000), "3 ticks * TICK_SECS(6s) = 18s");
        assert_eq!(e.ready_at_ms, Some(e.since_ms + 18_000));
    }

    #[test]
    fn a_resisted_cast_on_the_target_shows_up_failed_with_no_duration() {
        let ing = run(&[
            "[Tue Jul 28 15:01:00 2026] You hit a rat for 5 points of damage.",
            "[Tue Jul 28 15:01:05 2026] a rat resisted your Tashania!",
        ]);
        let dto = target_effects(&ing);
        let e = dto
            .effects
            .iter()
            .find(|e| e.spell == "Tashania")
            .expect("Tashania should be tracked even though it resisted");
        assert!(!e.landed);
        assert_eq!(e.duration_ms, None);
        assert_eq!(e.ready_at_ms, None);
    }

    /// why: Spencer's own ask -- "only show the highest for a skill
    /// line". Real spell "Tashania" has no rank II entry of its own in
    /// the catalog, so ingest::split_cast_rank treats "Tashania II" as
    /// an observed rank of the same line, not a separate spell.
    #[test]
    fn only_the_highest_rank_of_a_spell_line_shows_not_a_stale_lower_one() {
        let ing = run(&[
            "[Tue Jul 28 15:01:00 2026] You hit a rat for 5 points of damage.",
            "[Tue Jul 28 15:01:05 2026] a rat resisted your Tashania II!",
            "[Tue Jul 28 15:01:10 2026] a rat resisted your Tashania!",
        ]);
        let dto = target_effects(&ing);
        let tashania_rows: Vec<_> = dto
            .effects
            .iter()
            .filter(|e| e.spell.starts_with("Tashania"))
            .collect();
        assert_eq!(
            tashania_rows.len(),
            1,
            "one row for the whole Tashania line, not one per rank"
        );
        assert_eq!(
            tashania_rows[0].spell, "Tashania II",
            "the higher rank wins even though the plain-rank attempt was more recent"
        );
    }

    /// why: real bug, caught live against Spencer's own log -- a
    /// resisted "Wandering Mind VI" (rank text survives in a resist
    /// line) followed, minutes later, by a real successful land
    /// (attributed generically via recent_casts, which never carries a
    /// rank suffix) used to get stuck showing the stale resist forever,
    /// because the landed observation's unresolvable rank read as
    /// "known lower" under rank-only comparison. A fresh landing must
    /// always supersede an older failure.
    #[test]
    fn a_later_land_beats_an_earlier_resist_even_when_its_own_rank_is_unknown() {
        let ing = run(&[
            "[Tue Jul 28 15:01:00 2026] You hit a rat for 5 points of damage.",
            "[Tue Jul 28 15:01:02 2026] You begin casting Wandering Mind VI.",
            "[Tue Jul 28 15:01:05 2026] a rat resisted your Wandering Mind VI!",
            "[Tue Jul 28 15:01:07 2026] You hit a rat for 5 points of damage.",
            "[Tue Jul 28 15:01:09 2026] You begin casting Wandering Mind VI.",
            "[Tue Jul 28 15:01:11 2026] a rat stares off into space.",
        ]);
        let dto = target_effects(&ing);
        let rows: Vec<_> = dto
            .effects
            .iter()
            .filter(|e| e.spell.starts_with("Wandering Mind"))
            .collect();
        assert_eq!(rows.len(), 1, "one row for the whole line");
        assert!(
            rows[0].landed,
            "the later land must win, not the stale resist"
        );
        assert_eq!(rows[0].duration_ms, Some(120_000), "real 2-minute duration");
    }

    #[test]
    fn a_slain_target_clears_the_panel() {
        let ing = run_to_closure(&[
            "[Tue Jul 28 15:01:00 2026] You hit a rat for 5 points of damage.",
            "[Tue Jul 28 15:01:05 2026] You have slain a rat!",
        ]);
        let dto = target_effects(&ing);
        assert_eq!(
            dto.target, None,
            "a closed (slain) encounter reports no target"
        );
        assert!(dto.effects.is_empty());
    }

    /// why: Spencer's other named clear condition -- a teammate's charm
    /// flips the target's own allegiance to ally, no longer "the enemy"
    #[test]
    fn a_target_charmed_by_a_teammate_clears_the_panel() {
        let ing = run(&[
            "[Tue Jul 28 15:01:00 2026] You hit a rat for 5 points of damage.",
            "[Tue Jul 28 15:01:05 2026] a rat has been charmed.",
        ]);
        let dto = target_effects(&ing);
        assert_eq!(
            dto.target, None,
            "a charmed target is no longer an enemy to report effects against"
        );
    }

    /// why: a later resist must overwrite an earlier landing for the same spell
    #[test]
    fn the_most_recent_observation_wins_over_an_earlier_one() {
        let ing = run(&[
            "[Tue Jul 28 15:01:00 2026] You hit a rat for 5 points of damage.",
            "[Tue Jul 28 15:01:05 2026] a rat resisted your Tashania!",
            "[Tue Jul 28 15:01:10 2026] a rat resisted your Tashania!",
        ]);
        let dto = target_effects(&ing);
        let e = dto.effects.iter().find(|e| e.spell == "Tashania").unwrap();
        // why: 15:02:05 in epoch ms -- just confirming the later of the two wins, not the exact value
        assert!(!e.landed);
    }
}
