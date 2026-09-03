//! why: DPM/DPS calculator for the Spellbook's damage-spell auto-suggest.
//! Parses each spell's `slots` effect text (151 real DoT candidates,
//! never hand-curated) into a hit amount + shape, layers observed rank on top.
//!
//! - Nuke damage scales +6% compounding per rank level (I-X) --
//!   corrected per the game's own upgrade system (player-confirmed:
//!   "6% damage per level in addition to other things"). The earlier
//!   +10% linear/tier figure (measured 10.04%/tier off Ice Comet rank
//!   4->9->10) conflated that 6% base with the "other things" stacked
//!   on top of it; the raw measurement wasn't wrong, its attribution
//!   was. Cast time/mana stay at catalog values.
//! - A DoT's per-tick damage does NOT scale with rank; only a one-time
//!   "on cast" component does. Duration/cast time/mana still scale per
//!   wiki DoT-category rates (unverified, labeled separately).
//! - DoT tick interval assumed `TICK_SECS` (6s), unconfirmed.
//! - A multi-wave AE nuke (Frost Storm + ~24 siblings) follows the DoT
//!   rule on reuse (recasting extends/resets it), the nuke rule on
//!   damage (every wave scales with rank). Recast floors at casting
//!   time, not the catalog's `recast_time` (too short to actually fire).
//! - A DoT's duration IS its reuse cadence -- `dps_ignoring_reuse` must
//!   not divide the whole multi-tick lifetime by casting time. See
//!   `DamageSpellDto::dps_ignoring_reuse`.
//! - Every number is an average that folds in AA crits, not a per-hit
//!   simulation. Direct-damage AAs (crit chance) and DoT AAs (crit
//!   damage) use separate rates -- see `crit_multiplier`'s doc. Only
//!   counts an AA actually confirmed-purchased this session
//!   (`Ingest::aa`); a rank never bought contributes 0%.

use crate::spelldata::{self, Spell, SpellClass};
use regex::Regex;
use serde::Serialize;
use std::sync::OnceLock;

/// why: 6% compounding per rank level (I-X), not 10% linear -- see module doc
pub const RANK_DAMAGE_PER_LEVEL: f64 = 0.06;
pub const TICK_SECS: f64 = 6.0;

// why: unverified, wiki-sourced -- see this module's own doc for why
// these three (and only these three) still borrow the eqlwiki "Spell
// Upgrade System" guide's Damage-over-Time-category numbers, unlike the
// damage rate above which was corrected against real measurement.
pub const DOT_CAST_TIME_PER_TIER: f64 = -0.04;
pub const DOT_MANA_PER_TIER: f64 = -0.02;
pub const DOT_DURATION_PER_TIER: f64 = 0.05;

// why: real AA catalog values (packs/aa.json), index 0 = rank 1. Direct-
// damage and DoT crits are separate AA pairs with separate numbers --
// DoTs get a much bigger damage bonus (Destructive Cascade) than direct
// damage spells do (Destructive Fury), confirmed against the catalog,
// not assumed symmetric.
const FURY_OF_MAGIC_CHANCE: [f64; 4] = [0.02, 0.04, 0.07, 0.10];
const DESTRUCTIVE_FURY_BONUS: [f64; 3] = [0.30, 0.60, 1.00];
const CRITICAL_AFFLICTION_CHANCE: [f64; 3] = [0.03, 0.06, 0.09];
const DESTRUCTIVE_CASCADE_BONUS: [f64; 3] = [1.25, 1.50, 1.75];

fn aa_rank_of(ing: &crate::ingest::Ingest, name: &str) -> u8 {
    ing.aa
        .all()
        .filter(|(_, g)| g.name == name)
        .map(|(_, g)| g.rank)
        .max()
        .unwrap_or(0)
}

fn table_value(table: &[f64], rank: u8) -> f64 {
    rank.checked_sub(1)
        .and_then(|i| table.get(i as usize))
        .copied()
        .unwrap_or(0.0)
}

/// why: expected-value fold-in, not a per-hit roll -- `1 + chance*bonus`
/// is the average multiplier a spell's damage sees over many casts,
/// whether the AA rolls once per cast (direct damage) or once per tick
/// (Critical Affliction's own wording, "at each 6 second interval") --
/// expectation is linear either way, so the same formula covers both,
/// just fed a different (chance, bonus) AA pair per caller.
fn crit_multiplier(
    ing: &crate::ingest::Ingest,
    chance_aa: &str,
    chance_table: &[f64],
    bonus_aa: &str,
    bonus_table: &[f64],
) -> f64 {
    let chance = table_value(chance_table, aa_rank_of(ing, chance_aa));
    let bonus = table_value(bonus_table, aa_rank_of(ing, bonus_aa));
    1.0 + chance * bonus
}

fn hit_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)Decrease (?:Current )?Hit ?Points by ([\d,]+)(?:\s*\(L\d+\))?(?:\s+to\s+([\d,]+)\s*\(L\d+\))?(\s+per\s+tick)?",
        )
        .unwrap()
    })
}

fn upfront_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)Decrease HP when cast by ([\d,]+)").unwrap())
}

fn parse_num(s: &str) -> f64 {
    s.replace(',', "").parse().unwrap_or(0.0)
}

/// why: (hit_amount, is_dot, upfront_amount); None if no recognizable
/// damage effect. Takes the highest-level value for a leveled range --
/// this app only deals with level-50 characters.
fn parse_damage(spell: &Spell) -> Option<(f64, bool, f64)> {
    let mut hit = 0.0_f64;
    let mut is_dot = false;
    let mut upfront = 0.0_f64;
    let mut found = false;
    for slot in &spell.slots {
        if let Some(caps) = hit_re().captures(&slot.effect) {
            found = true;
            let a = parse_num(&caps[1]);
            let b = caps.get(2).map(|m| parse_num(m.as_str()));
            let amount = b.map_or(a, |b| a.max(b));
            if amount > hit {
                hit = amount;
            }
            if caps.get(3).is_some() {
                is_dot = true;
            }
        }
        if let Some(caps) = upfront_re().captures(&slot.effect) {
            upfront += parse_num(&caps[1]);
        }
    }
    found.then_some((hit, is_dot, upfront))
}

fn wave_count_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)\b(?:x)?(\d+|one|two|three|four|five|six)\s*waves?\b").unwrap()
    })
}

fn word_to_num(w: &str) -> Option<f64> {
    match w.to_ascii_lowercase().as_str() {
        "one" => Some(1.0),
        "two" => Some(2.0),
        "three" => Some(3.0),
        "four" => Some(4.0),
        "five" => Some(5.0),
        "six" => Some(6.0),
        _ => w.parse().ok(),
    }
}

/// why: AE nuke catalog damage is per-wave; confirmed against the real
/// log (Frost Storm) -- up to 4 real hit lines from one cast on one
/// target, typically 2-3. Wiki says "three waves", one short of the
/// observed max -- uses the spell's own stated count, undercount stated not fudged.
fn parse_wave_count(description: &str) -> Option<f64> {
    wave_count_re()
        .captures(description)
        .and_then(|c| word_to_num(&c[1]))
}

fn ticks_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)(\d+)\s*ticks?").unwrap())
}
fn hours_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)(\d+)\s*hours?").unwrap())
}
fn minutes_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)(\d+(?:\.\d+)?)\s*min(?:ute)?s?").unwrap())
}
fn seconds_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)(\d+)\s*(?:sec(?:ond)?s?|s)\b").unwrap())
}

/// why: None for Permanent/Unlimited or unrecognized shapes. Leveled
/// range takes the last (highest-level) segment, re-parsed fresh since
/// units can differ between range ends.
fn parse_duration_secs(raw: &str) -> Option<f64> {
    let s = raw.trim();
    if s.eq_ignore_ascii_case("instant") {
        return Some(0.0);
    }
    if s.eq_ignore_ascii_case("permanent") || s.eq_ignore_ascii_case("unlimited") {
        return None;
    }
    let last = s.rsplit(" to ").next().unwrap_or(s);
    let last = last.split('@').next().unwrap_or(last).trim();

    let mut total = 0.0;
    let mut matched = false;
    if let Some(c) = ticks_re().captures(last) {
        total += c[1].parse::<f64>().unwrap_or(0.0) * TICK_SECS;
        matched = true;
    } else {
        if let Some(c) = hours_re().captures(last) {
            total += c[1].parse::<f64>().unwrap_or(0.0) * 3600.0;
            matched = true;
        }
        if let Some(c) = minutes_re().captures(last) {
            total += c[1].parse::<f64>().unwrap_or(0.0) * 60.0;
            matched = true;
        }
        if let Some(c) = seconds_re().captures(last) {
            total += c[1].parse::<f64>().unwrap_or(0.0);
            matched = true;
        }
    }
    matched.then_some(total)
}

#[derive(Debug, Clone, Serialize)]
pub struct DamageSpellDto {
    pub name: String,
    pub icon: Option<String>,
    pub classes: Vec<SpellClass>,
    /// why: what this spell's own damage checks against (e.g. "Cold
    /// (-10)"); lets a caller line a debuff's own decreased-resist
    /// types up against what the character's rotation actually needs
    pub resist: Option<String>,
    pub is_dot: bool,
    /// why: observed live rank this session, 0 means no evidence yet not "unranked"
    pub rank: u8,
    /// why: None for a nuke; DoT duration in seconds, rank-independent
    pub duration_secs: Option<f64>,
    pub mana: f64,
    pub casting_time: f64,
    pub recast_time: f64,
    /// why: spells sharing one reuse timer lock together in the rotation
    /// -- "timer:<id>" from the install's own spell file (spelltimers),
    /// None when the spell has no shared timer or the file is absent
    pub reuse_group: Option<String>,
    /// why: full rank-adjusted damage from one application
    pub total_damage: f64,
    /// why: instant portion of total_damage -- all of it for a nuke,
    /// just the "on cast" component for a DoT; exposed for callers
    /// rescaling on top (e.g. Invocation toggle)
    pub instant_damage: f64,
    pub dpm: f64,
    /// why: cast + wait recast; for a DoT, "recast" is really its own duration
    pub dps_with_reuse: f64,
    /// why: damage per second of casting time, no reuse wait. For a
    /// nuke this is total_damage/casting_time. NOT for a DoT --
    /// crediting the whole tick-stream here would double-count damage
    /// already captured by `dps_with_reuse`'s own cycle; only the
    /// one-time instant component counts, the metric for "worth an
    /// instant cast", not "worth maintaining" (that's `dps_with_reuse`).
    pub dps_ignoring_reuse: f64,
}

fn build_dto(
    spell: &Spell,
    rank: u8,
    dd_crit_mult: f64,
    dot_crit_mult: f64,
    timer: Option<u32>,
) -> Option<DamageSpellDto> {
    let (base_hit, is_dot, base_upfront) = parse_damage(spell)?;
    if base_hit <= 0.0 {
        return None;
    }
    // why: compounding, not linear -- each rank level multiplies the
    // previous, per the upgrade system's own 6%/level rule (module doc)
    let hit_mult = (1.0 + RANK_DAMAGE_PER_LEVEL).powi(rank as i32);
    // why: upfront always gets the verified rate (mechanically a hit);
    // per-tick only does for a nuke, not a DoT -- see module doc. Gets
    // the direct-damage crit treatment for the same reason (a hit, not a tick).
    let upfront = base_upfront * hit_mult * dd_crit_mult;

    let base_mana = spell.mana.unwrap_or(0.0);
    let base_casting_time = spell.casting_time.unwrap_or(0.0);
    let base_recast_time = spell.recast_time.unwrap_or(0.0).max(0.0);

    let (total_damage, instant_damage, duration_secs, mana, casting_time, recast_time, cycle_secs) =
        if is_dot {
            let base_dur = spell.duration.as_deref().and_then(parse_duration_secs)?;
            if base_dur <= 0.0 {
                return None;
            }
            let r = rank as f64;
            let mana = (base_mana * (1.0 + DOT_MANA_PER_TIER * r)).max(1.0);
            let casting_time = (base_casting_time * (1.0 + DOT_CAST_TIME_PER_TIER * r)).max(0.1);
            let dur = base_dur * (1.0 + DOT_DURATION_PER_TIER * r);
            let ticks = (dur / TICK_SECS).round().max(1.0);
            // why: base_hit unscaled by rank -- the flat-per-tick
            // correction; dot_crit_mult folds in Critical Affliction/
            // Destructive Cascade's expected-value bump (see module doc)
            let total = base_hit * dot_crit_mult * ticks + upfront;
            // why: only the one-time "on cast" burst is instant value from
            // this press; the tick stream is already accounted separately
            // via dps_with_reuse's own duration-bound cycle
            (
                total,
                upfront,
                Some(dur),
                mana,
                casting_time,
                base_recast_time,
                dur.max(casting_time + base_recast_time),
            )
        } else {
            // why: dd_crit_mult folds in Fury of Magic/Destructive Fury's expected-value bump (see module doc)
            let hit = base_hit * hit_mult * dd_crit_mult;
            let mana = base_mana.max(1.0);
            let casting_time = base_casting_time.max(0.1);
            // why: AE catalog damage is per-wave, see `parse_wave_count`.
            // No multi-target splash modeled (single-target throughout),
            // but a lone target really eats every wave, confirmed against
            // the real log. Each wave is its own full hit, not a tick.
            let waves = match spell.target_type.as_deref() {
                Some("Targeted AE") | Some("PB AE") => spell
                    .description
                    .as_deref()
                    .and_then(parse_wave_count)
                    .unwrap_or(1.0),
                _ => 1.0,
            };
            let total = hit * waves + upfront;
            // why: a multi-wave spell follows the DoT "can't spam it" rule
            // -- waves keep landing for real seconds after cast (confirmed
            // on Frost Storm) but catalog recast_time is a token 1.5s. No
            // wiki "safe to recast" field, so this floors recast at the
            // cast time itself, a stated conservative estimate.
            let recast_time = if waves > 1.0 {
                base_recast_time.max(casting_time)
            } else {
                base_recast_time
            };
            // why: a nuke's damage is all instant, nothing deferred
            (
                total,
                total,
                None,
                mana,
                casting_time,
                recast_time,
                casting_time + recast_time,
            )
        };

    // why: the game's own timer id (spelltimers) -- spells sharing one
    // lock together in the rotation; no shape heuristics
    let reuse_group = timer.map(|t| format!("timer:{t}"));
    Some(DamageSpellDto {
        name: spell.name.clone(),
        icon: spell.icon.clone(),
        classes: spell.classes.clone(),
        resist: spell.resist.clone(),
        is_dot,
        reuse_group,
        rank,
        duration_secs,
        mana,
        casting_time,
        recast_time,
        total_damage,
        instant_damage,
        dpm: total_damage / mana,
        dps_with_reuse: total_damage / cycle_secs,
        dps_ignoring_reuse: instant_damage / casting_time,
    })
}

/// why: every damage-capable spell, rank-adjusted; unfiltered by
/// class/level -- caller already has that filtering logic.
/// `assume_max_rank`: substitutes a flat rank 10 for every spell instead
/// of this session's observed rank -- a "what would be best once maxed"
/// preview, reusing the same verified scaling math.
pub fn list_damage_spells(
    ing: &crate::ingest::Ingest,
    assume_max_rank: bool,
    base_dir: Option<&std::path::Path>,
) -> Vec<DamageSpellDto> {
    let timers = base_dir.map(crate::spelltimers::timers);
    let dd_crit_mult = crit_multiplier(
        ing,
        "Fury of Magic",
        &FURY_OF_MAGIC_CHANCE,
        "Destructive Fury",
        &DESTRUCTIVE_FURY_BONUS,
    );
    let dot_crit_mult = crit_multiplier(
        ing,
        "Critical Affliction",
        &CRITICAL_AFFLICTION_CHANCE,
        "Destructive Cascade",
        &DESTRUCTIVE_CASCADE_BONUS,
    );
    spelldata::spells()
        .iter()
        .filter_map(|s| {
            let rank = if assume_max_rank {
                10
            } else {
                ing.spell_ranks.rank_of(&s.name).unwrap_or(0)
            };
            build_dto(
                s,
                rank,
                dd_crit_mult,
                dot_crit_mult,
                timers
                    .as_ref()
                    .and_then(|t| crate::spelltimers::timer_of(t, &s.name)),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_nuke_effect_parses_its_flat_amount() {
        assert_eq!(
            parse_damage_test("Decrease Hitpoints by 808"),
            Some((808.0, false, 0.0))
        );
    }

    #[test]
    fn a_leveled_range_takes_the_highest_value() {
        assert_eq!(
            parse_damage_test("Decrease Hitpoints by 2 (L1) to 51 (L50)"),
            Some((51.0, false, 0.0))
        );
    }

    #[test]
    fn a_per_tick_leveled_range_is_flagged_as_a_dot() {
        assert_eq!(
            parse_damage_test("Decrease Hitpoints by 54 (L1) to 90 (L50) per tick"),
            Some((90.0, true, 0.0))
        );
    }

    #[test]
    fn hit_points_with_a_space_and_current_prefix_both_parse() {
        assert_eq!(
            parse_damage_test("Decrease Current Hit Points by 71"),
            Some((71.0, false, 0.0))
        );
        assert_eq!(
            parse_damage_test("Decrease Hit Points by 154"),
            Some((154.0, false, 0.0))
        );
    }

    #[test]
    fn a_non_damage_effect_parses_to_none() {
        assert_eq!(parse_damage_test("Decrease AC by 3"), None);
    }

    fn parse_damage_test(effect: &str) -> Option<(f64, bool, f64)> {
        let spell = Spell {
            id: "t".into(),
            name: "t".into(),
            url: None,
            description: None,
            classes: vec![],
            skill: None,
            mana: None,
            range: None,
            casting_time: None,
            fizzle_time: None,
            recast_time: None,
            duration: None,
            target_type: None,
            spell_type: None,
            resist: None,
            msg_cast_on_you: None,
            msg_cast_on_other: None,
            msg_wears_off: None,
            slots: vec![crate::spelldata::SpellSlot {
                slot: 1,
                effect: effect.to_string(),
            }],
            items_with_effect: vec![],
            where_to_obtain: None,
            era: None,
            categories: vec![],
            icon: None,
        };
        parse_damage(&spell)
    }

    #[test]
    fn duration_shapes_parse_to_seconds() {
        assert_eq!(parse_duration_secs("Instant"), Some(0.0));
        assert_eq!(parse_duration_secs("Permanent"), None);
        assert_eq!(parse_duration_secs("36 Sec"), Some(36.0));
        assert_eq!(parse_duration_secs("7 ticks"), Some(42.0));
        assert_eq!(parse_duration_secs("1 Min 24 Sec"), Some(84.0));
        assert_eq!(parse_duration_secs("1 min 24s"), Some(84.0));
        assert_eq!(
            parse_duration_secs("1 ticks @L1 to 5 ticks @L5"),
            Some(30.0)
        );
        assert_eq!(
            parse_duration_secs("1 ticks @L1 to 1.5 minutes @L50"),
            Some(90.0)
        );
    }

    #[test]
    fn wave_counts_parse_from_real_description_wordings() {
        assert_eq!(
            parse_wave_count(
                "Calls down a frost storm that falls in three waves, causing between 250 damage"
            ),
            Some(3.0)
        );
        assert_eq!(
            parse_wave_count("causing 675 damage (x3 waves?) to several creatures"),
            Some(3.0)
        );
        assert_eq!(
            parse_wave_count("causing 1-3 waves of 540 damage to 1-4 creatures"),
            Some(3.0)
        );
        assert_eq!(
            parse_wave_count(
                "causing between 193 and 216 damage to several creatures, without waves"
            ),
            None
        );
        assert_eq!(
            parse_wave_count("Creates a wave of intense color around you"),
            None
        );
    }

    fn make_spell(
        slots_effects: &[&str],
        duration: Option<&str>,
        target_type: Option<&str>,
        description: Option<&str>,
    ) -> Spell {
        Spell {
            id: "t".into(),
            name: "t".into(),
            url: None,
            description: description.map(str::to_string),
            classes: vec![],
            skill: None,
            mana: Some(100.0),
            range: None,
            casting_time: Some(2.0),
            fizzle_time: None,
            recast_time: Some(1.0),
            duration: duration.map(str::to_string),
            target_type: target_type.map(str::to_string),
            spell_type: None,
            resist: None,
            msg_cast_on_you: None,
            msg_cast_on_other: None,
            msg_wears_off: None,
            slots: slots_effects
                .iter()
                .enumerate()
                .map(|(i, e)| crate::spelldata::SpellSlot {
                    slot: i as u32 + 1,
                    effect: e.to_string(),
                })
                .collect(),
            items_with_effect: vec![],
            where_to_obtain: None,
            era: None,
            categories: vec![],
            icon: None,
        }
    }

    /// why: the timer id is the only source -- Spencer named Spike/Spear
    /// of Disease/Spear of Pain and the rains as sharing, and the game
    /// file agrees; nothing is inferred from shape
    #[test]
    fn the_reuse_group_is_the_spell_files_timer_id_and_nothing_else() {
        let rain = spelldata::spell_by_name("Frost Storm").expect("in pack");
        let nuke = spelldata::spell_by_name("Conflagration").expect("in pack");
        assert_eq!(
            build_dto(rain, 0, 1.0, 1.0, Some(3)).map(|d| d.reuse_group),
            Some(Some("timer:3".to_string()))
        );
        assert_eq!(
            build_dto(rain, 0, 1.0, 1.0, None).map(|d| d.reuse_group),
            Some(None)
        );
        assert_eq!(
            build_dto(nuke, 0, 1.0, 1.0, None).map(|d| d.reuse_group),
            Some(None)
        );
    }

    #[test]
    fn a_nukes_dps_ignoring_reuse_is_its_full_damage_over_casting_time() {
        let spell = make_spell(
            &["Decrease Hitpoints by 100"],
            Some("Instant"),
            Some("Single"),
            None,
        );
        let dto = build_dto(&spell, 0, 1.0, 1.0, None).unwrap();
        assert_eq!(dto.total_damage, 100.0);
        assert_eq!(dto.dps_ignoring_reuse, 100.0 / 2.0); // total damage / casting_time
    }

    #[test]
    fn a_dots_dps_ignoring_reuse_excludes_the_tick_stream_direct_correction() {
        // why: the exact bug reported -- crediting a DoT's whole multi-
        // tick lifetime total to "no reuse" DPS fabricates an absurd
        // number by double-counting damage that ticks out on its own.
        let spell = make_spell(
            &["Decrease Hitpoints by 50 per tick"],
            Some("36 Sec"),
            Some("Single"),
            None,
        );
        let dto = build_dto(&spell, 0, 1.0, 1.0, None).unwrap();
        assert!(dto.is_dot);
        assert_eq!(dto.total_damage, 300.0); // 6 ticks * 50, the real lifetime total
                                             // why: must NOT be total_damage/casting_time (150.0); no upfront, so 0
        assert_eq!(dto.dps_ignoring_reuse, 0.0);
    }

    #[test]
    fn a_dot_with_an_upfront_component_only_counts_that_part_as_instant() {
        let spell = make_spell(
            &[
                "Decrease HP when cast by 40",
                "Decrease Hitpoints by 50 per tick",
            ],
            Some("36 Sec"),
            Some("Single"),
            None,
        );
        let dto = build_dto(&spell, 0, 1.0, 1.0, None).unwrap();
        assert!(dto.is_dot);
        assert_eq!(dto.total_damage, 340.0); // 40 upfront + 6*50 ticks
                                             // why: only the upfront component, over casting_time
        assert_eq!(dto.dps_ignoring_reuse, 40.0 / 2.0);
    }

    #[test]
    fn crit_multiplier_is_neutral_with_no_aa_purchased() {
        let ing = crate::ingest::Ingest::default();
        assert_eq!(
            crit_multiplier(
                &ing,
                "Fury of Magic",
                &FURY_OF_MAGIC_CHANCE,
                "Destructive Fury",
                &DESTRUCTIVE_FURY_BONUS
            ),
            1.0
        );
    }

    #[test]
    fn crit_multiplier_matches_hand_computed_expectation_at_real_max_rank() {
        // why: real catalog max ranks (packs/aa.json) -- Fury of Magic 4/4
        // (10% chance), Destructive Fury 3/3 (+100% dmg) -> 1 + .10*1.00 = 1.10
        let mut ing = crate::ingest::Ingest::default();
        ing.aa.observe(0, "Fury of Magic".into(), 4, 0);
        ing.aa.observe(0, "Destructive Fury".into(), 3, 0);
        let mult = crit_multiplier(
            &ing,
            "Fury of Magic",
            &FURY_OF_MAGIC_CHANCE,
            "Destructive Fury",
            &DESTRUCTIVE_FURY_BONUS,
        );
        assert!((mult - 1.10).abs() < 1e-9);

        // why: Critical Affliction 2/3 (6%), Destructive Cascade 1/3 (+125%) -> 1 + .06*1.25 = 1.075
        ing.aa.observe(0, "Critical Affliction".into(), 2, 0);
        ing.aa.observe(0, "Destructive Cascade".into(), 1, 0);
        let dot_mult = crit_multiplier(
            &ing,
            "Critical Affliction",
            &CRITICAL_AFFLICTION_CHANCE,
            "Destructive Cascade",
            &DESTRUCTIVE_CASCADE_BONUS,
        );
        assert!((dot_mult - 1.075).abs() < 1e-9);
    }

    #[test]
    fn aa_rank_of_takes_the_highest_observed_rank_not_the_last() {
        // why: a character's log shows every purchase, e.g. rank 1 then
        // rank 2 -- must report the max, not whichever was observed last
        let mut ing = crate::ingest::Ingest::default();
        ing.aa.observe(0, "Fury of Magic".into(), 1, 0);
        ing.aa.observe(1, "Fury of Magic".into(), 2, 0);
        assert_eq!(aa_rank_of(&ing, "Fury of Magic"), 2);
    }

    #[test]
    fn build_dto_applies_the_crit_multiplier_to_nuke_and_dot_damage() {
        let nuke = make_spell(
            &["Decrease Hitpoints by 100"],
            Some("Instant"),
            Some("Single"),
            None,
        );
        let dto = build_dto(&nuke, 0, 1.10, 1.0, None).unwrap();
        assert!((dto.total_damage - 110.0).abs() < 1e-9);

        let dot = make_spell(
            &["Decrease Hitpoints by 50 per tick"],
            Some("36 Sec"),
            Some("Single"),
            None,
        );
        let dto = build_dto(&dot, 0, 1.0, 1.075, None).unwrap();
        assert!((dto.total_damage - 322.5).abs() < 1e-9); // 6 ticks * 50 * 1.075
    }
}
