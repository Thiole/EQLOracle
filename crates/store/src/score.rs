//! Scoring one parse against an expected-value baseline, per ability.
//!
//! The question this answers: "given what this ability actually does on
//! average, is this parse's result better or worse than that, and by how
//! much" -- which separates rotation/skill quality from crit luck far more
//! honestly than comparing one parse's raw total to another's (see the
//! parse-analyzer discussion this equation came out of: a small sample of
//! kills can't tell a good rotation from a lucky one on its own, but an
//! ability's own long-run mean, taken from `by_ability` over the whole
//! store rather than one fight, can).
//!
//! ## The gear seam
//!
//! `GearModifiers` exists and does nothing yet -- every field defaults to
//! 1.0 (no effect), because inventory/item detection doesn't exist in this
//! codebase (see `BACKLOG.md`). That is not the same claim as "gear doesn't
//! matter". A focus effect that boosts spell damage or crit chance changes
//! what "expected" should mean for that cast, and pretending otherwise
//! would make a geared parse look like it's simply outperforming its
//! baseline for no reason. Rather than hardcode that blindness into the
//! formula shape, `GearModifiers` is threaded through explicitly as a
//! multiplicative seam: once gear detection exists, it populates these
//! fields from real detected foci instead of the caller inventing a number,
//! and every call site downstream (`score_ability`, `score_parse`) already
//! expects the parameter and needs no restructuring. Until then, callers
//! pass `GearModifiers::default()` and the formula degrades exactly to
//! "expected = the ability's own observed average", which is still a real,
//! useful baseline on its own.

use crate::ability::AbilityId;
use crate::query::AbilityRow;
use std::collections::HashMap;

/// Multiplicative modifiers layered on top of an ability's observed
/// baseline. See the module doc for why these all default to neutral.
#[derive(Debug, Clone, Copy)]
pub struct GearModifiers {
    /// A damage-amplification focus (e.g. "Increase Spell Damage by N%").
    pub damage_focus: f64,
    /// A crit-chance-boosting focus. Not yet consumed by `score_ability`
    /// below -- `AbilityRow` doesn't expose a separate expected-crit-rate
    /// baseline distinct from its blended mean yet, so this field is here
    /// as part of the seam's shape but has no effect until that baseline
    /// exists. Kept rather than omitted so the struct's shape doesn't
    /// change (and every call site with it) the day it does.
    pub crit_chance_focus: f64,
    /// Same reasoning, for a crit-damage-boosting focus.
    pub crit_damage_focus: f64,
}

impl Default for GearModifiers {
    fn default() -> Self {
        GearModifiers {
            damage_focus: 1.0,
            crit_chance_focus: 1.0,
            crit_damage_focus: 1.0,
        }
    }
}

/// Expected vs. observed for one ability within whatever selection `actual`
/// was scoped to (a parse, an encounter, a trailing window).
#[derive(Debug, Clone, Copy)]
pub struct AbilityScore {
    pub ability: AbilityId,
    pub hits: u64,
    pub observed_total: u64,
    /// `baseline.mean() * hits * gear.damage_focus` -- what this many
    /// landed hits "should" have totalled, at the ability's own long-run
    /// observed average, adjusted by whatever gear signal is available.
    pub expected_total: f64,
    /// `observed_total / expected_total`. Above 1.0 means this selection
    /// ran hot against its own baseline (crit luck, or a real advantage
    /// the baseline doesn't know about yet); below 1.0 means it ran cold.
    ///
    /// This scores landed-hit quality only. A fizzled or resisted cast
    /// never produces a damage row at all, so it is not in `hits` and
    /// cannot drag this ratio down by itself -- pair with
    /// `eqlp_session::cast::Resolver`'s outcome breakdown to see attempt
    /// success rate, which this number does not and cannot capture.
    pub ratio: f64,
}

fn score_ability(baseline: &AbilityRow, actual: &AbilityRow, gear: &GearModifiers) -> AbilityScore {
    let expected_per_hit = baseline.mean() * gear.damage_focus;
    let expected_total = expected_per_hit * actual.hits as f64;
    let ratio = if expected_total > 0.0 {
        actual.total as f64 / expected_total
    } else {
        0.0
    };
    AbilityScore {
        ability: actual.ability,
        hits: actual.hits,
        observed_total: actual.total,
        expected_total,
        ratio,
    }
}

/// Every ability in `actual` scored against its matching entry in
/// `baseline`, plus an aggregate. An ability with no baseline entry at all
/// (never observed outside this selection -- nothing to compare against
/// yet) is skipped rather than scored against zero, which would make a
/// brand-new ability look like an infinite overperformance.
#[derive(Debug, Clone)]
pub struct ParseScore {
    pub per_ability: Vec<AbilityScore>,
    pub observed_total: u64,
    pub expected_total: f64,
    /// Sum of observed over sum of expected -- not an average of each
    /// ability's own ratio, so one high-sample-size ability isn't drowned
    /// out by several low-sample-size ones weighing in equally.
    pub ratio: f64,
}

pub fn score_parse(
    baseline: &[AbilityRow],
    actual: &[AbilityRow],
    gear: &GearModifiers,
) -> ParseScore {
    let base_by_id: HashMap<AbilityId, &AbilityRow> =
        baseline.iter().map(|r| (r.ability, r)).collect();
    let mut per_ability = Vec::new();
    let mut observed_total = 0u64;
    let mut expected_total = 0.0f64;
    for row in actual {
        if let Some(base) = base_by_id.get(&row.ability) {
            let s = score_ability(base, row, gear);
            observed_total += s.observed_total;
            expected_total += s.expected_total;
            per_ability.push(s);
        }
    }
    per_ability.sort_by_key(|b| std::cmp::Reverse(b.observed_total));
    let ratio = if expected_total > 0.0 {
        observed_total as f64 / expected_total
    } else {
        0.0
    };
    ParseScore {
        per_ability,
        observed_total,
        expected_total,
        ratio,
    }
}
