//! DPM/DPS calculator for the Spellbook builder's damage-spell auto-
//! suggest feature. Parses each catalog spell's own `slots` effect text
//! (never hand-curated -- 151 real DoT-shaped candidates alone, far too
//! many to list by hand) into a usable hit amount + damage shape, then
//! layers this session's own observed live rank on top.
//!
//! Two things this is explicit about rather than silently assuming:
//!
//! - **A nuke's damage scales as `damage * (1 + RANK_DAMAGE_PER_TIER *
//!   rank)`** -- +10% of *base* (unranked) damage per live in-game rank
//!   tier, not the eqlwiki "Spell Upgrade System" guide page's stated 6%
//!   compounding-per-tier. That page self-flags as still being tested
//!   and possibly inaccurate; a real, gear-controlled measurement
//!   against this character's own log landed on +10%/tier instead --
//!   Ice Comet's rank climbed 4 -> 9 -> 10 in a single 19-second
//!   mote-redemption burst, so damage sampled in a tight window either
//!   side of that moment couldn't have been confounded by a gear/AA
//!   change: (1321.7 - 834.9) / 808 base damage / 6 tiers = 10.04% of
//!   base per tier, near-exact. Only damage is corrected this way for a
//!   nuke -- cast time/mana cost are left at their catalog (unranked)
//!   values, since those specific numbers were never independently
//!   checked the same way and the wiki's own figures for them are
//!   exactly as unconfirmed as the damage one turned out to be wrong.
//! - **A DoT is different, per direct correction**: its *per-tick*
//!   damage does **not** scale with rank at all -- only a one-time
//!   "when cast" burst component (if the spell has one) gets the same
//!   verified `RANK_DAMAGE_PER_TIER` treatment a nuke's hit does, since
//!   that component is mechanically an instant hit, not a tick. Cast
//!   time, mana cost, and duration *do* still shrink/grow with rank for
//!   a DoT -- unlike the damage number, these three were never disputed,
//!   so the wiki's own stated Damage-over-Time-category rates are used
//!   as a labeled, unverified best guess (`DOT_CAST_TIME_PER_TIER` etc.
//!   below), clearly distinct from the one number that's actually been
//!   measured. Explicitly not handling exceptions to the flat-per-tick
//!   rule ("things like Ice Storm" were named as one, unconfirmed which
//!   catalog entry that is or how it differs) -- every DoT here is
//!   treated the same way rather than guessing at a special case.
//! - **DoT tick interval is assumed to be `TICK_SECS` (6 seconds)** --
//!   the standard across this whole genre, not independently confirmed
//!   for this specific game. Stated, not hidden.
//! - **A multi-wave AE nuke (Frost Storm and its ~24 siblings) follows
//!   the DoT rule on reuse, not the nuke rule** -- per direct
//!   correction: it "isn't independent, recasting just extends the
//!   effect/resets it", but "essentially fully scale[s] with rank since
//!   every hit mimics [the] first hit". So its damage stays on the nuke
//!   side (every wave gets the full verified rank multiplier, not a
//!   flat DoT-tick amount), but its *reuse* is treated like a DoT's own
//!   duration-bound cycle -- recast is floored at the spell's own
//!   casting time (a stated, conservative *estimate* giving the wave
//!   sequence room to resolve, since there's no wiki-stated "safe to
//!   recast" time to read this from), not the catalog's short
//!   `recast_time`, which real log evidence shows can't actually be
//!   spammed for fresh volleys.
//! - **A DoT's own duration already IS its "no reuse" cadence** --
//!   direct correction: `dps_ignoring_reuse` must not divide a DoT's
//!   *whole* multi-tick lifetime total by casting time (that fabricates
//!   an absurd number by double-counting damage the DoT ticks out on
//!   its own regardless of what's cast next). For a DoT this field
//!   holds only its one-time instant/"on cast" component (0 for most
//!   real DoTs, which have none) over casting time -- see
//!   `DamageSpellDto::dps_ignoring_reuse`'s own doc for the full
//!   reasoning and what it's actually for.

use crate::spelldata::{self, Spell, SpellClass};
use regex::Regex;
use serde::Serialize;
use std::sync::OnceLock;

pub const RANK_DAMAGE_PER_TIER: f64 = 0.10;
pub const TICK_SECS: f64 = 6.0;

// why: unverified, wiki-sourced -- see this module's own doc for why
// these three (and only these three) still borrow the eqlwiki "Spell
// Upgrade System" guide's Damage-over-Time-category numbers, unlike the
// damage rate above which was corrected against real measurement.
pub const DOT_CAST_TIME_PER_TIER: f64 = -0.04;
pub const DOT_MANA_PER_TIER: f64 = -0.02;
pub const DOT_DURATION_PER_TIER: f64 = 0.05;

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

/// Reads every damage-shaped slot line -- `(hit_amount, is_dot,
/// upfront_amount)`, `None` if this spell has no recognizable damage
/// effect at all (a buff/CC/summon/etc., or a wording this parser
/// doesn't cover -- stays out of the candidate list rather than guess).
/// `hit_amount` is the catalog's own highest-level value when the wiki
/// states a level-scaled range ("2 (L1) to 51 (L50)") -- this app only
/// ever deals with level-50 characters, per `MAX_CHARACTER_LEVEL`.
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

/// A "Targeted AE"/"PB AE" nuke's own catalog damage is per-*wave*, and
/// several fall in multiple waves that all land on a single target when
/// nothing else is in range to spread across -- confirmed directly
/// against the real log (Frost Storm, restricted to proper-named/unique
/// targets so a same-named second mob can't be mistaken for a repeat
/// hit): up to 4 real "You hit ... by Frost Storm" lines from *one*
/// cast on one target, typically 2-3 (resist chance per wave presumably
/// explains the variance). The wiki's own description states "three
/// waves" for Frost Storm specifically -- one short of the observed
/// max -- so this uses each spell's *own* stated wave count (sourced,
/// spell-specific) rather than a blanket guess, with that known
/// possible undercount stated rather than silently corrected by an
/// invented fudge factor.
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

/// Parses a wiki `duration` string into total seconds -- `None` for
/// "Permanent"/"Unlimited" (doesn't fit a refresh-cadence model at all)
/// or a shape this parser doesn't recognize. A leveled range
/// ("N ticks @L1 to M minutes @L50") takes its *last* segment (the
/// highest level, same stance `parse_damage` takes on damage ranges) --
/// units can differ between the two ends of a range, so the segment is
/// re-parsed fresh rather than assumed to match the first one's unit.
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
    pub is_dot: bool,
    /// This session's own observed live rank -- 0 if never cast this
    /// session (not "unranked" fact, just "no evidence yet").
    pub rank: u8,
    /// `None` for a nuke (instant); a DoT's own duration in seconds,
    /// rank-independent (see this module's own doc on what rank scaling
    /// does and doesn't touch).
    pub duration_secs: Option<f64>,
    pub mana: f64,
    pub casting_time: f64,
    pub recast_time: f64,
    /// Full damage from one application, rank-adjusted -- a nuke's
    /// single hit, or a DoT's (per-tick * tick count) + any one-time
    /// "when cast" component.
    pub total_damage: f64,
    /// The portion of `total_damage` that's genuinely instant -- equal
    /// to `total_damage` for a nuke; for a DoT, just its one-time "on
    /// cast" component (0 for the large majority, which have none).
    /// This is `dps_ignoring_reuse`'s own numerator, exposed separately
    /// so a caller applying its own damage modifier on top (the
    /// Spellbook builder's Invocation toggle, e.g.) can rescale this the
    /// same proportional way without needing to know which effect-text
    /// shape produced it.
    pub instant_damage: f64,
    pub dpm: f64,
    /// Cast it, then wait out its own recast timer before casting again
    /// -- for a DoT, "recast" is really "how long until it's worth
    /// refreshing", which is its own duration (or the cast+recast time
    /// if that's somehow longer).
    pub dps_with_reuse: f64,
    /// No reuse wait at all -- damage per second of *casting time*
    /// spent, as if you could always weave straight into your next
    /// spell. For a nuke this is `total_damage / casting_time`, same as
    /// `dps_with_reuse` minus the recast wait. **For a DoT it is NOT**
    /// -- direct correction: a DoT already ticks on its own regardless
    /// of what's cast next, so crediting the whole multi-tick lifetime
    /// total here would double-count damage that lands anyway (already
    /// captured via `dps_with_reuse`'s own duration-bound cycle). Only
    /// the one-time "on cast" instant component (`upfront` -- 0 for the
    /// large majority of DoTs, which have none) divided by casting time
    /// is genuinely *this button press's* instant value, so that's what
    /// this field holds for a DoT. This is also what the auto-suggest
    /// rotation compares against a nuke's own rate to decide whether a
    /// DoT is worth casting at all *as an instant hit* -- it is
    /// deliberately not the metric for "is this DoT worth maintaining
    /// over its full duration", which is a separate question answered
    /// by `dps_with_reuse` instead.
    pub dps_ignoring_reuse: f64,
}

fn build_dto(spell: &Spell, rank: u8) -> Option<DamageSpellDto> {
    let (base_hit, is_dot, base_upfront) = parse_damage(spell)?;
    if base_hit <= 0.0 {
        return None;
    }
    let hit_mult = 1.0 + RANK_DAMAGE_PER_TIER * rank as f64;
    // why: the upfront/instant component always gets the verified rate
    // (it's mechanically a hit, DoT or not); the per-tick amount only
    // does for a nuke -- for a real DoT it's a direct correction that it
    // must NOT scale with rank at all, see this module's own doc.
    let upfront = base_upfront * hit_mult;

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
            // why: base_hit, unscaled -- the flat-per-tick correction.
            let total = base_hit * ticks + upfront;
            // why: direct correction -- a DoT already has its own "no reuse
            // needed" cadence (it ticks on its own regardless of what's cast
            // next), so crediting the *whole* multi-tick lifetime total to
            // "damage per second of casting time, ignoring reuse" double-
            // counts damage that was going to land anyway. Only the one-time
            // "on cast" burst (if any -- most DoTs have none, and correctly
            // read as 0 here, not a wrong answer) is genuinely instant value
            // from *this* button press; the tick stream is accounted for
            // separately via `dps_with_reuse`'s own duration-bound cycle.
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
            let hit = base_hit * hit_mult;
            let mana = base_mana.max(1.0);
            let casting_time = base_casting_time.max(0.1);
            // why: a "Targeted AE"/"PB AE" nuke's catalog damage is per-wave
            // -- see `parse_wave_count`'s own doc. Multi-target splash
            // isn't modeled (this whole calculator assumes single-target
            // play throughout), but a lone target really does eat every
            // wave, confirmed against the real log. Every wave gets the
            // full verified rank multiplier -- unlike a DoT's flat per-tick
            // amount, each wave is its own full hit, not a tick.
            let waves = match spell.target_type.as_deref() {
                Some("Targeted AE") | Some("PB AE") => spell
                    .description
                    .as_deref()
                    .and_then(parse_wave_count)
                    .unwrap_or(1.0),
                _ => 1.0,
            };
            let total = hit * waves + upfront;
            // why: direct correction -- a multi-wave spell follows the same
            // "can't just spam it" rule a DoT does. Real log evidence: waves
            // keep landing for several real seconds after cast (confirmed
            // on Frost Storm, restricted to proper-named targets so a
            // same-named second mob couldn't fake a repeat hit), yet the
            // catalog's own `recast_time` is a token 1.5s -- recasting on
            // that short a timer wouldn't fire a fresh, independent volley,
            // it would reset/extend the wave sequence still resolving from
            // the *previous* cast. There's no wiki-stated "how long until
            // it's actually safe to recast" field to read this from (the
            // spell's own `duration` is just "Instant"), so this uses a
            // stated, conservative *estimate* -- recast is treated as no
            // shorter than the cast itself, giving the wave sequence
            // roughly its own cast time's worth of room to resolve before
            // a recast is credited as a fresh volley rather than a wasted
            // reset. Explicitly an estimate, not a measured number, unlike
            // the wave *count* and the rank-damage rate above.
            let recast_time = if waves > 1.0 {
                base_recast_time.max(casting_time)
            } else {
                base_recast_time
            };
            // why: a nuke's damage is *all* instant -- nothing deferred to
            // account for separately, unlike a DoT.
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

    Some(DamageSpellDto {
        name: spell.name.clone(),
        icon: spell.icon.clone(),
        classes: spell.classes.clone(),
        is_dot,
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

/// Every catalog spell with a recognizable damage effect, rank-adjusted
/// against this session's own observed casts (`ing.spell_ranks`) --
/// unfiltered by class/level, same stance the rest of the Spellbook
/// picker takes (the caller already has the class/level-cap filtering
/// logic and this shouldn't duplicate it a second, driftable way).
pub fn list_damage_spells(ing: &crate::ingest::Ingest) -> Vec<DamageSpellDto> {
    spelldata::spells()
        .iter()
        .filter_map(|s| {
            let rank = ing.spell_ranks.rank_of(&s.name).unwrap_or(0);
            build_dto(s, rank)
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

    #[test]
    fn a_nukes_dps_ignoring_reuse_is_its_full_damage_over_casting_time() {
        let spell = make_spell(
            &["Decrease Hitpoints by 100"],
            Some("Instant"),
            Some("Single"),
            None,
        );
        let dto = build_dto(&spell, 0).unwrap();
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
        let dto = build_dto(&spell, 0).unwrap();
        assert!(dto.is_dot);
        assert_eq!(dto.total_damage, 300.0); // 6 ticks * 50, the real lifetime total
                                             // dps_ignoring_reuse must NOT be total_damage / casting_time (that
                                             // would be 150.0) -- no upfront component here, so it's 0.
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
        let dto = build_dto(&spell, 0).unwrap();
        assert!(dto.is_dot);
        assert_eq!(dto.total_damage, 340.0); // 40 upfront + 6*50 ticks
        assert_eq!(dto.dps_ignoring_reuse, 40.0 / 2.0); // only the upfront component, over casting_time
    }
}
