//! why: the focus effects on the gear you are wearing, as the DPS model
//! sees them -- read from the newest inventory dump, the items pack's
//! `effects.focus` name, and the focus spell's own slot text in
//! packs/spells.json ("Increase Spell Damage by 1% to 20%", "Limit Max
//! Level: 60 (lose 5% per level after)", "Limit Type: Detrimental" ...).
//!
//! A "1% to 20%" focus rolls its bonus per cast (Spencer: "UP TO, so it
//! rolls a random damage bonus"), so the model uses the expected value,
//! the middle of the range. Whether the roll happens at all on a given
//! cast ("when it glows") is not in any data we have; `ACTIVATION` is
//! that assumption in one place, 1.0 until measured.
//!
//! Only the best applicable focus of each kind counts, the game's own
//! rule. A spell's level for the "Limit Max Level" decay is the lowest
//! level any of its classes gets it at -- the favourable read when the
//! casting class isn't known.

use crate::spelldata::{self, Spell};
use serde::Serialize;
use std::path::Path;

/// why: see module doc -- the one knob for "does the focus fire this cast"
pub const ACTIVATION: f64 = 1.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum FocusKind {
    /// "Increase Spell Damage by a% to b%"
    Damage,
    /// "Increase Spell Haste by n%" -- cast time cut
    Haste,
    /// "Decrease Spell Mana Cost by a% to b%"
    ManaCost,
    /// "Increase Spell Duration by n%"
    Duration,
}

#[derive(Debug, Clone, Serialize)]
pub struct FocusEffect {
    /// why: the worn item it comes from, for the UI
    pub item: String,
    pub name: String,
    pub kind: FocusKind,
    /// why: percent range; a fixed "by 30%" is lo == hi
    pub lo: f64,
    pub hi: f64,
    pub max_level: Option<u32>,
    /// why: fraction lost per spell level past `max_level` (0.05 = "lose 5% per level after")
    pub decay_per_level: f64,
    pub detrimental_only: bool,
    pub beneficial_only: bool,
    pub min_duration_secs: Option<f64>,
    pub max_duration_secs: Option<f64>,
    pub min_casting_time: Option<f64>,
    pub exclude_ae: bool,
    /// why: "Limit Effect: Current HP" -- damage or heal spells only
    pub current_hp_only: bool,
    /// why: any other "Limit Effect: X" (Summon Skeleton Pet, Summon Pet,
    /// ...) restricts the focus to spells carrying that effect -- none of
    /// which deal damage, so the DPS model never applies it (real:
    /// Reanimation Haste II was cutting nuke casts by 30% before this)
    pub other_effect_only: bool,
}

impl FocusEffect {
    /// why: the expected percent, before any level decay
    pub fn expected_pct(&self) -> f64 {
        (self.lo + self.hi) / 2.0 * ACTIVATION
    }
}

fn pct_range(text: &str, prefix: &str) -> Option<(f64, f64)> {
    let rest = text.strip_prefix(prefix)?;
    let nums: Vec<f64> = rest
        .split(|c: char| !c.is_ascii_digit() && c != '.')
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse().ok())
        .collect();
    match nums.as_slice() {
        [a] => Some((*a, *a)),
        [a, b, ..] => Some((*a, *b)),
        _ => None,
    }
}

fn secs_of(text: &str) -> Option<f64> {
    text.trim().trim_end_matches('s').trim().parse::<f64>().ok()
}

/// why: a focus spell's slots -> one effect; None when the spell isn't
/// a focus the DPS model understands (pet power, range, reagents ...)
pub fn parse_focus(item: &str, spell: &Spell) -> Option<FocusEffect> {
    let mut out: Option<FocusEffect> = None;
    let blank = |kind, lo, hi| FocusEffect {
        item: item.to_string(),
        name: spell.name.clone(),
        kind,
        lo,
        hi,
        max_level: None,
        decay_per_level: 0.0,
        detrimental_only: false,
        beneficial_only: false,
        min_duration_secs: None,
        max_duration_secs: None,
        min_casting_time: None,
        exclude_ae: false,
        current_hp_only: false,
        other_effect_only: false,
    };
    for slot in &spell.slots {
        let t = slot.effect.trim();
        if let Some((lo, hi)) = pct_range(t, "Increase Spell Damage by ") {
            out = Some(blank(FocusKind::Damage, lo, hi));
        } else if let Some((lo, hi)) = pct_range(t, "Increase Spell Haste by ") {
            out = Some(blank(FocusKind::Haste, lo, hi));
        } else if let Some((lo, hi)) = pct_range(t, "Decrease Spell Mana Cost by ") {
            out = Some(blank(FocusKind::ManaCost, lo, hi));
        } else if let Some((lo, hi)) = pct_range(t, "Increase Spell Duration by ") {
            out = Some(blank(FocusKind::Duration, lo, hi));
        }
    }
    let mut f = out?;
    for slot in &spell.slots {
        let t = slot.effect.trim();
        if let Some(rest) = t.strip_prefix("Limit Max Level: ") {
            let mut it = rest.split(|c: char| !c.is_ascii_digit());
            f.max_level = it.next().and_then(|n| n.parse().ok());
            if let Some(p) = rest.split("lose ").nth(1) {
                f.decay_per_level = p
                    .split('%')
                    .next()
                    .and_then(|n| n.trim().parse::<f64>().ok())
                    .map(|n| n / 100.0)
                    .unwrap_or(0.0);
            }
        } else if t == "Limit Type: Detrimental" {
            f.detrimental_only = true;
        } else if t == "Limit Type: Beneficial" {
            f.beneficial_only = true;
        } else if let Some(rest) = t.strip_prefix("Limit Min Duration: ") {
            f.min_duration_secs = secs_of(rest);
        } else if let Some(rest) = t.strip_prefix("Limit Max Duration: ") {
            f.max_duration_secs = secs_of(rest);
        } else if let Some(rest) = t.strip_prefix("Limit Min Casting Time: ") {
            f.min_casting_time = secs_of(rest);
        } else if t.starts_with("Limit Target: Exclude") && (t.contains("AE") || t.contains("PB")) {
            f.exclude_ae = true;
        } else if t == "Limit Effect: Current HP" {
            f.current_hp_only = true;
        } else if let Some(rest) = t.strip_prefix("Limit Effect: ") {
            if !rest.starts_with("Exclude") {
                f.other_effect_only = true;
            }
        }
    }
    Some(f)
}

/// What one spell looks like to a focus's limits.
#[derive(Debug, Clone, Copy)]
pub struct SpellShape {
    pub level: Option<u32>,
    pub detrimental: bool,
    pub duration_secs: f64,
    pub casting_time: f64,
    pub is_ae: bool,
    pub deals_damage: bool,
}

/// why: the multiplier this one focus gives `shape`, None when a limit
/// excludes it. Damage/Duration: 1 + pct; Haste/ManaCost: 1 - pct.
pub fn multiplier(f: &FocusEffect, shape: &SpellShape) -> Option<f64> {
    if f.detrimental_only && !shape.detrimental {
        return None;
    }
    if f.beneficial_only && shape.detrimental {
        return None;
    }
    if f.current_hp_only && !shape.deals_damage {
        return None;
    }
    if f.other_effect_only {
        return None;
    }
    if f.exclude_ae && shape.is_ae {
        return None;
    }
    if let Some(min) = f.min_duration_secs {
        if shape.duration_secs < min {
            return None;
        }
    }
    if let Some(max) = f.max_duration_secs {
        if shape.duration_secs > max {
            return None;
        }
    }
    if let Some(min) = f.min_casting_time {
        if shape.casting_time < min {
            return None;
        }
    }
    let mut pct = f.expected_pct() / 100.0;
    if let (Some(max), Some(level)) = (f.max_level, shape.level) {
        if level > max {
            pct *= (1.0 - f.decay_per_level * (level - max) as f64).max(0.0);
        }
    }
    if pct <= 0.0 {
        return None;
    }
    Some(match f.kind {
        FocusKind::Damage | FocusKind::Duration => 1.0 + pct,
        FocusKind::Haste | FocusKind::ManaCost => (1.0 - pct).max(0.0),
    })
}

/// why: the game applies only the best focus of a kind -- the strongest
/// applicable multiplier, 1.0 when none applies
pub fn best<'a>(
    effects: &'a [FocusEffect],
    kind: FocusKind,
    shape: &SpellShape,
) -> (f64, Option<&'a FocusEffect>) {
    let mut best: (f64, Option<&FocusEffect>) = (1.0, None);
    for f in effects.iter().filter(|f| f.kind == kind) {
        if let Some(m) = multiplier(f, shape) {
            let stronger = match kind {
                FocusKind::Damage | FocusKind::Duration => m > best.0,
                FocusKind::Haste | FocusKind::ManaCost => m < best.0,
            };
            if stronger {
                best = (m, Some(f));
            }
        }
    }
    best
}

/// why: the item name as worn ("Crimson Robe of Alendine +6") -> the
/// pack's base name
fn base_item_name(name: &str) -> &str {
    match name.rsplit_once(" +") {
        Some((base, tier)) if tier.chars().all(|c| c.is_ascii_digit()) => base,
        _ => name,
    }
}

/// why: every focus on the gear in the newest inventory dump; empty when
/// there is no dump, an item isn't in the pack, or its focus isn't a
/// kind the model understands
pub fn equipped(base_dir: &Path) -> Vec<FocusEffect> {
    let Some((file, _)) = crate::inventory::find_existing_dump(base_dir) else {
        return Vec::new();
    };
    let Ok(path) = crate::inventory::dump_path(base_dir, &file) else {
        return Vec::new();
    };
    let Ok(inv) = crate::inventory::parse(&path) else {
        return Vec::new();
    };
    // why: a focus reaches you two ways -- on the worn item itself, or
    // through an exaltation socketed into it ("Back-Slot7 White
    // Dragonscale Cloak (Exaltation)" carries that cloak's Improved
    // Damage III; Spencer: "if the inventory has the foci in, use it,
    // the game handles that logic"). Which sockets an item may take is
    // the game's rule, already applied by the time the dump is written.
    let mut sources: Vec<String> = inv.equipped.values().map(|i| i.name.clone()).collect();
    for sockets in inv.exalted.values() {
        for source in sockets.values() {
            sources.push(format!("{source} (Exaltation)"));
        }
    }
    let mut out = Vec::new();
    for worn in sources {
        let base = base_item_name(worn.trim_end_matches(" (Exaltation)"));
        let Some(pack_item) = crate::itemdata::items()
            .iter()
            .find(|i| i.name == base || i.name == worn)
        else {
            continue;
        };
        let Some(focus_name) = pack_item
            .effects
            .get("focus")
            .and_then(|e| e.get("name"))
            .and_then(|n| n.as_str())
        else {
            continue;
        };
        let Some(spell) = spelldata::spell_by_name(focus_name) else {
            continue;
        };
        if let Some(f) = parse_focus(&worn, spell) {
            out.push(f);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape(level: u32, detrimental: bool, dur: f64, cast: f64, ae: bool) -> SpellShape {
        SpellShape {
            level: Some(level),
            detrimental,
            duration_secs: dur,
            casting_time: cast,
            is_ae: ae,
            deals_damage: true,
        }
    }

    /// why: the real catalog focus -- "1% to 20%" reads as its middle,
    /// instant detrimental damage only, decaying past level 60
    #[test]
    fn improved_damage_iii_is_an_expected_ten_and_a_half_percent_on_a_nuke() {
        let s = spelldata::spell_by_name("Improved Damage III").expect("in pack");
        let f = parse_focus("White Dragonscale Cloak", s).expect("parses");
        assert_eq!(f.kind, FocusKind::Damage);
        assert_eq!((f.lo, f.hi), (1.0, 20.0));
        assert_eq!(f.max_level, Some(60));
        assert!(f.detrimental_only && f.current_hp_only && f.exclude_ae);
        let nuke = shape(50, true, 0.0, 5.0, false);
        assert!((multiplier(&f, &nuke).unwrap() - 1.105).abs() < 1e-9);
        // why: Max Duration 0s -- a DoT is out; an AE is out; level 62 decays 10%
        assert_eq!(multiplier(&f, &shape(50, true, 60.0, 5.0, false)), None);
        assert_eq!(multiplier(&f, &shape(50, true, 0.0, 5.0, true)), None);
        let decayed = multiplier(&f, &shape(62, true, 0.0, 5.0, false)).unwrap();
        assert!((decayed - (1.0 + 0.105 * 0.9)).abs() < 1e-9);
    }

    /// why: the DoT-side sibling -- Min Duration 24s keeps it off nukes
    #[test]
    fn burning_affliction_applies_to_dots_not_nukes() {
        let s = spelldata::spell_by_name("Burning Affliction III").expect("in pack");
        let f = parse_focus("test", s).expect("parses");
        assert_eq!(multiplier(&f, &shape(40, true, 0.0, 3.0, false)), None);
        assert!(multiplier(&f, &shape(40, true, 60.0, 3.0, false)).is_some());
    }

    /// why: only the best of a kind counts, never a stack
    #[test]
    fn only_the_best_focus_of_a_kind_applies() {
        let a = parse_focus(
            "a",
            spelldata::spell_by_name("Improved Damage I").expect("in pack"),
        )
        .unwrap();
        let b = parse_focus(
            "b",
            spelldata::spell_by_name("Improved Damage III").expect("in pack"),
        )
        .unwrap();
        let both = [a.clone(), b.clone()];
        let (m, which) = best(&both, FocusKind::Damage, &shape(50, true, 0.0, 5.0, false));
        // why: real pack -- I and III share "1% to 20%"; at level 50 the
        // tier I focus (max level 20) has decayed to nothing, III is whole
        assert_eq!(which.map(|f| f.name.as_str()), Some("Improved Damage III"));
        assert!((m - (1.0 + b.expected_pct() / 100.0)).abs() < 1e-9);
        assert_eq!(multiplier(&a, &shape(50, true, 0.0, 5.0, false)), None);
    }

    /// why: real regression -- a pet-haste focus must not touch a nuke
    #[test]
    fn a_focus_limited_to_another_effect_never_applies_to_damage() {
        let s = spelldata::spell_by_name("Reanimation Haste II").expect("in pack");
        let f = parse_focus("legs", s).expect("parses");
        assert_eq!(f.kind, FocusKind::Haste);
        assert!(f.other_effect_only);
        assert_eq!(multiplier(&f, &shape(40, true, 0.0, 5.0, false)), None);
    }

    #[test]
    fn a_worn_items_tier_suffix_is_stripped_for_the_pack_lookup() {
        assert_eq!(
            base_item_name("Crimson Robe of Alendine +6"),
            "Crimson Robe of Alendine"
        );
        assert_eq!(base_item_name("Blade of Abrogation"), "Blade of Abrogation");
    }
}
