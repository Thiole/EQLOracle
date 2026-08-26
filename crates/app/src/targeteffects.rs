//! why: Skill Tracker's target-effects section -- Spencer's own ask:
//! "tracking should be done per target, so dots can be easily tracked
//! per target ... a target (ex: Lord Nagafen) that shows the icons for
//! tracked spell effects that were/tried on him, like slow with a
//! timer." Scoped to the player's OWN engagement with a target, not
//! combat::current_encounter's own "most recently ACTIVE, whole
//! store" resolution -- real bug, caught live, twice: a pure debuff
//! cast never opens the damage graph at all (see target_sym's own
//! doc), and in group content, current_encounter keeps returning
//! whichever mob a PARTY MEMBER (not necessarily "You") is actively
//! hitting, starving out whatever "You" are personally casting on or
//! being attacked by -- "some mobs its not detecting... it only
//! happened when I am attacked, not when I am casting". Resolved
//! instead from two player-scoped signals directly (see target_sym's
//! own doc), whichever is more recent.
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
//!
//! Observation here stays unfiltered -- every real DoT/debuff the
//! player lands or attempts on the target, not just tracked ones --
//! so a spell added to the tracked list mid-fight shows its real
//! history immediately. Spencer's correction (twice, now): which of
//! those get DISPLAYED is player-selected, but a SEPARATE list from
//! skill_status/cooldowns' own tracked_skills -- "dont do spell
//! tracking for 'ready' ... maybe we need a separate list for 'per
//! target', not a tracking effect like charm etc since thats not a
//! per target thing". preferences.rs's own tracked_target_effects,
//! not tracked_skills; a spell added there never gets its own
//! cooldown/READY row, only ever shows up here. That filter lives
//! client-side (SkillTrackerWidget.svelte), same split as skill_status/
//! cooldowns already has, just against the other list now.

use crate::combat;
use crate::ingest::Ingest;
use crate::{spelldata, spelleffect};
use eqlp_session::{Allegiance, State};
use eqlp_source::Millis;
use eqlp_store::{tag, EventKind, Sym};
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

/// why: don't resurrect a target no one's actually still engaging --
/// same window either signal uses
const TARGET_STALE_MS: Millis = 5 * 60 * 1000;

/// why: two real, independent player-scoped signals for "who am I
/// currently fighting", whichever is more recent:
/// - melee/avoided: the most recent Damage or Miss row with "You" on
///   EITHER side -- covers being attacked (a mob's own miss/hit
///   against you is real, unambiguous evidence of who you're fighting,
///   even with zero casting at all) and covers you personally landing/
///   missing a swing. A full backward scan of the store, same pattern
///   combat::current_encounter already uses -- early-terminates near
///   the end for any real live-tail session, so it's cheap in practice
///   despite scanning "the whole log" in principle.
/// - cast/effect: `Ingest::effects`' own most-recent You-sourced ping
///   (`Effects::most_recent_by_you`) -- covers a pure debuff/CC cast
///   that lands no damage at all (Tashania, a resist-decrease debuff),
///   which the melee/avoided signal above can never see on its own.
///
/// Deliberately NOT combat::current_encounter -- its own "most recently
/// ACTIVE, whole store" resolution is exactly right for a DPS meter,
/// but wrong here: real bug, caught live, group content -- whichever
/// mob a PARTY MEMBER (not necessarily "You") is actively hitting kept
/// winning that scan, so the panel tracked the group's own current
/// punching bag instead of whatever "You" were personally casting
/// debuffs on or being attacked by.
fn target_sym(ing: &Ingest, now: Millis) -> Option<Sym> {
    let melee = ing.store.names.get("You").and_then(|you_sym| {
        (0..ing.store.len()).rev().find_map(|i| {
            if !matches!(ing.store.kind[i], EventKind::Damage | EventKind::Miss) {
                return None;
            }
            let other = if ing.store.actor[i] == you_sym {
                Some(ing.store.target[i])
            } else if ing.store.target[i] == you_sym {
                Some(ing.store.actor[i])
            } else {
                None
            };
            other.map(|sym| (sym, ing.store.ts[i]))
        })
    });
    let cast = ing
        .effects
        .most_recent_by_you()
        .map(|(entity, p)| (Sym(entity), p.ts));

    let (sym, ts) = match (melee, cast) {
        (Some(m), Some(c)) => {
            if m.1 >= c.1 {
                m
            } else {
                c
            }
        }
        (Some(m), None) => m,
        (None, Some(c)) => c,
        (None, None) => return None,
    };
    (now - ts <= TARGET_STALE_MS).then_some(sym)
}

/// why: the overlay's own poll -- same polled-on-tick shape as
/// combat::live_meter/effects::status_effects
pub fn target_effects(ing: &Ingest) -> TargetEffectsDto {
    let now = ing.now_ms();
    let Some(target_sym) = target_sym(ing, now) else {
        return TargetEffectsDto::default();
    };
    // why: the encounter (if any) THIS specific target belongs to, not
    // combat::current_encounter's own group-wide "most recently active
    // any" pick -- used below only to bound the DoT-tick scan, not to
    // decide who the target is
    let enc = combat::encounter_for(ing, target_sym);

    let target_name = ing.store.name(target_sym).to_string();
    let state = ing
        .timeline
        .state_at(target_sym.0, now)
        .map(|(s, _)| s)
        .unwrap_or(State::Engaged);
    // why: Charmed is checked directly and first, always honored --
    // Spencer's own named clear condition, and the one real way a
    // fought mob legitimately becomes an ally mid-session
    if state == State::Charmed {
        return TargetEffectsDto::default();
    }
    // why: Allegiance::of doesn't special-case Dead (a dead Unproven
    // mob still reads Enemy by kind alone) -- a lingering melee/cast
    // signal could still name an already-confirmed-dead entity
    if state == State::Dead {
        return TargetEffectsDto::default();
    }
    // why: real bug, caught live -- Entities::kind is a global, sticky,
    // name-keyed classification (`note_shared_target`'s own doc: "same
    // sticky-forever mechanism as chat proof"), and a real mob can get
    // misclassified Kind::Player forever from one coincidental edge
    // case (two mobs cross-damaging the same anchor, a reflected hit,
    // ...) -- a real haunted chest, actively biting/bashing "You" right
    // now, silently cleared here even though dps meter showed it fine
    // (live_meter never checks kind at all). If this target is the
    // OPEN encounter's own anchor -- link()'s own real, damage-verified
    // "which side is the mob" choice, not the sticky classification --
    // that's authoritative and the stale kind lookup is skipped
    // entirely. Only falls back to Allegiance::of when there's no such
    // encounter yet (the pure-debuff-cast fallback path in target_sym).
    let confirmed_enemy_anchor = enc.is_some_and(|e| e.target == target_sym && e.is_open());
    if !confirmed_enemy_anchor {
        let kind = ing.encounters.entities.kind(&target_name);
        if !Allegiance::of(kind, state).is_enemy() {
            return TargetEffectsDto::default();
        }
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

    // why: enc, from encounter_for above -- the specific target's own
    // encounter, not combat::current_encounter's group-wide pick. None
    // when no real Damage event ever named this target at all (a pure
    // debuff resolved only through the melee/cast fallback in
    // target_sym) -- provably nothing to find here either way.
    if let (Some(enc), Some(you_sym)) = (enc, ing.store.names.get("You")) {
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
    // why: real bug, caught live -- Ingest::effects is name-keyed
    // (Sym), unbounded, whole-session history, same as the timeline/
    // entities lookups above. A common respawning mob name ("a haunted
    // chest", 14,764 real occurrences in one real log) means `all()`
    // returns effects from EVERY past spawn ever fought, not just this
    // one -- an 18-day-old Tashania observation showed up as if it
    // were live on a mob that only spawned moments ago. Scoped to the
    // same TARGET_STALE_MS window target_sym's own resolution already
    // uses, so only this encounter's real activity counts.
    for p in ing.effects.all(target_sym.0) {
        if now - p.ts > TARGET_STALE_MS {
            continue;
        }
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

    /// why: real bug, caught live against Spencer's own log -- a pure
    /// debuff/CC cast with no damage component at all (real "Tashania",
    /// a resist-decrease debuff) never opens or extends
    /// combat::current_encounter (damage-graph only, by design). A
    /// support/CC character casting on a mob but never personally
    /// dealing damage used to get no target at all, ever -- "im not
    /// seeing a target pop up in the overlay". Zero Damage events
    /// anywhere in this scenario, on purpose.
    #[test]
    fn a_pure_debuff_with_no_damage_at_all_still_resolves_a_target() {
        let ing = run(&["[Tue Jul 28 15:01:00 2026] a rat resisted your Tashania!"]);
        let dto = target_effects(&ing);
        assert_eq!(
            dto.target.as_deref(),
            Some("a rat"),
            "the damage graph has nothing, but the fallback still finds a rat"
        );
        let e = dto
            .effects
            .iter()
            .find(|e| e.spell == "Tashania")
            .expect("Tashania should show even with zero damage ever exchanged");
        assert!(!e.landed);
    }

    /// why: real bug, caught live -- "when a spell lands, the timer
    /// isnt going up, as if it landed". Tashania's own third-person
    /// landing text ("Someone glances nervously about.") is shared
    /// catalog-wide by 8 real rank/typo variants of the same line
    /// (Tashan/Tashani/Tashania/Tashanian/Tashina/Wind of Tashani x2),
    /// so spelltext.rs's own global dictionary has to drop it as
    /// ambiguous -- but a real nearby "You begin casting Tashania"
    /// still resolves it locally (ingest::attribute_effect's own tier
    /// 3, extended to check msg_cast_on_other too, not just
    /// msg_cast_on_you/wears_off).
    #[test]
    fn a_landing_confirmed_only_through_an_ambiguous_third_person_line_still_attributes_to_you() {
        let ing = run(&[
            "[Tue Jul 28 15:01:00 2026] You begin casting Tashania.",
            "[Tue Jul 28 15:01:03 2026] a rat glances nervously about.",
        ]);
        let dto = target_effects(&ing);
        assert_eq!(dto.target.as_deref(), Some("a rat"));
        let e = dto
            .effects
            .iter()
            .find(|e| e.spell == "Tashania")
            .expect("a real land, even through an ambiguous line, should attribute to You");
        assert!(e.landed);
        assert_eq!(e.duration_ms, Some(660_000), "real 11-minute duration");
    }

    /// why: real bug, caught live, group content -- "some mobs its not
    /// detecting as combat... it only happened when I am attacked, not
    /// when I am casting". combat::current_encounter's own "most
    /// recently ACTIVE, whole store" resolution kept returning whatever
    /// mob a PARTY MEMBER was hitting, even though "You" were actually
    /// casting on a completely different one. target_sym is
    /// player-scoped now (melee/avoided OR cast/effect with "You" on
    /// one side), so a groupmate's own damage on an unrelated mob must
    /// never override it, even when it's chronologically later.
    #[test]
    fn a_party_members_own_damage_on_a_different_mob_doesnt_steal_the_target() {
        let ing = run(&[
            "[Tue Jul 28 15:01:00 2026] You begin casting Tashania.",
            "[Tue Jul 28 15:01:03 2026] a rat glances nervously about.",
            "[Tue Jul 28 15:02:00 2026] Groupmate hit a snake for 20 points of damage.",
        ]);
        let dto = target_effects(&ing);
        assert_eq!(
            dto.target.as_deref(),
            Some("a rat"),
            "a party member's own damage on an unrelated mob must not override what You are actually engaged with"
        );
    }

    /// why: real bug, caught live -- "a haunted chest, only thing in
    /// combat... it was parsing fine to dps meter" but not here.
    /// Entities::kind is a global, sticky, name-keyed classification
    /// (note_shared_target's own doc: "same sticky-forever mechanism as
    /// chat proof") -- a real mob can get misclassified Kind::Player
    /// forever from one coincidental edge (here: dealing damage to an
    /// anchor "You" had already confirmed against something else
    /// entirely), silently poisoning EVERY future encounter with that
    /// same name. dps meter never checks kind at all, so it kept
    /// working; target_effects' own Allegiance check didn't. A later,
    /// real, separate fight where the same-named mob is the OPEN
    /// encounter's own real damage-verified anchor overrides the stale
    /// classification.
    #[test]
    fn a_sticky_misclassified_mob_still_resolves_via_its_own_open_encounter() {
        let ing = run(&[
            "[Tue Jul 28 15:01:00 2026] You hit a bat for 5 points of damage.",
            "[Tue Jul 28 15:01:02 2026] a rat hit a bat for 3 points of damage.",
            // why: 13s later -- past Policy::default's own 10s idle_ms,
            // so the earlier bat encounter (and "a rat"'s membership in
            // it) is fully expired before this real, separate fight
            // opens a fresh one with "a rat" as its own real anchor
            "[Tue Jul 28 15:01:15 2026] You hit a rat for 10 points of damage.",
        ]);
        let dto = target_effects(&ing);
        assert_eq!(
            dto.target.as_deref(),
            Some("a rat"),
            "a real open encounter's own damage-verified anchor beats a stale, sticky Kind::Player misclassification"
        );
    }

    /// why: real bug, caught live, same haunted-chest report -- Ingest::
    /// effects is name-keyed too, unbounded whole-session history. A
    /// common respawning mob name accumulates effects from EVERY past
    /// spawn ever fought; an 18-day-old real observation showed up as
    /// if it were live on a mob that had only just spawned. Scoped to
    /// the same staleness window target_sym's own resolution already uses.
    #[test]
    fn a_stale_effect_observation_from_a_much_earlier_fight_is_excluded() {
        let ing = run(&[
            "[Tue Jul 28 15:01:00 2026] You begin casting Tashania.",
            "[Tue Jul 28 15:01:03 2026] a rat glances nervously about.",
            // why: 10 minutes later -- a real, later, separate fight
            // against the same-named mob
            "[Tue Jul 28 15:11:00 2026] You hit a rat for 5 points of damage.",
        ]);
        let dto = target_effects(&ing);
        assert_eq!(dto.target.as_deref(), Some("a rat"));
        assert!(
            dto.effects.iter().all(|e| e.spell != "Tashania"),
            "a 10-minute-stale observation from a much earlier fight shouldn't show as if active now"
        );
    }

    /// why: the fallback's own staleness cutoff -- a ping from ages ago
    /// shouldn't resurrect a target no one's actually still fighting
    #[test]
    fn a_stale_fallback_ping_past_the_window_reports_no_target() {
        let ing = run(&[
            "[Tue Jul 28 15:01:00 2026] a rat resisted your Tashania!",
            // why: 6 minutes later, past FALLBACK_TARGET_WINDOW_MS (5min)
            "[Tue Jul 28 15:07:00 2026] You tell your party, 'ready'",
        ]);
        let dto = target_effects(&ing);
        assert_eq!(dto.target, None);
    }

    /// why: Allegiance::of doesn't special-case Dead on its own (a dead
    /// Unproven mob still reads Enemy by kind alone) -- the fallback
    /// path needs its own explicit dead check, since there's no
    /// enc.is_open() to already rule this out the way the primary path has
    #[test]
    fn a_fallback_target_confirmed_dead_reports_no_target() {
        let ing = run(&[
            "[Tue Jul 28 15:01:00 2026] a rat resisted your Tashania!",
            "[Tue Jul 28 15:01:05 2026] You have slain a rat!",
        ]);
        let dto = target_effects(&ing);
        assert_eq!(dto.target, None);
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
