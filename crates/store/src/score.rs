//! why: scores a parse's ability totals against their own long-run mean
//!
//! ## The gear seam
//!
//! why: GearModifiers is a no-op seam until item detection exists

use crate::ability::AbilityId;
use crate::query::AbilityRow;
use std::collections::HashMap;

/// why: multiplicative modifiers, neutral until gear detection exists
#[derive(Debug, Clone, Copy)]
pub struct GearModifiers {
    pub damage_focus: f64,
    /// why: unused until AbilityRow exposes a separate crit-rate baseline
    pub crit_chance_focus: f64,
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

/// why: expected vs. observed for one ability, over whatever `actual` scopes
#[derive(Debug, Clone, Copy)]
pub struct AbilityScore {
    pub ability: AbilityId,
    pub hits: u64,
    pub observed_total: u64,
    /// why: what these hits "should" total at the ability's own average
    pub expected_total: f64,
    /// why: landed-hit quality only -- fizzles/resists never enter `hits`
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

/// why: no-baseline abilities are skipped, not scored against zero
#[derive(Debug, Clone)]
pub struct ParseScore {
    pub per_ability: Vec<AbilityScore>,
    pub observed_total: u64,
    pub expected_total: f64,
    /// why: sum-of-sums, not an average of ratios -- weights by sample size
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
