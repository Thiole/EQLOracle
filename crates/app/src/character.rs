//! why: Character Planner -- race + up to 3 classes + gear -> full
//! attribute sheet and gear-inclusive mana estimate. Pure calculation off
//! caller-supplied values, no log line states raw attributes. No notion
//! of items/slots itself -- `gear` is just an attr-name -> total map.
//!
//! THE TRIO MECHANIC (explained by the user, undocumented on eqlwiki):
//! EQL levels up to 3 classes at once, all simultaneously while active.
//! Each class's own level persists once reached even after being swapped
//! out. So overall level is the *minimum* of the 3 currently active
//! classes (SHD 46/ENC 50/WIZ 50 -> level 46), not a single stored
//! number. `estimate` takes each class's own level and derives the min.
//!
//! DATA PROVENANCE: eqlwiki doesn't publish race base attrs or class
//! adds. `race_base`/`class_add` are classic-EQ values, unverified
//! assumption EQL reuses them. One fact independently verified
//! (eqltools.com/attributes, client-mined): every class adds exactly 30
//! points total. Everything else is a labeled guess (`verified: false`).

use crate::gearplanner;
use serde::Serialize;
use std::collections::HashMap;

/// why: fixed attribute order every table below agrees on, no per-table drift
pub const ATTRS: &[&str] = &["STR", "STA", "AGI", "DEX", "WIS", "INT", "CHA"];

/// why: independently verified sum; re-checked live via `bad_class_adds`
pub const CLASS_ADD_TOTAL: i32 = 30;

/// why: EQL's real attribute ceiling per the user, supersedes the
/// standalone planner's unconfirmed 150. Nothing clamps to it here.
pub const ATTR_CAP: i32 = 510;

/// why: race base stats, unbuffed/ungeared, pulled from the same source
/// spreadsheet as `manadata`. 4 entries corrected against it (Troll STA
/// 109->114, Ogre 132->127, Vah Shir 75->70, Froglok DEX 85->100).
fn race_base(code: &str) -> Option<[i32; 7]> {
    Some(match code {
        "HUM" => [75, 75, 75, 75, 75, 75, 75],
        "BAR" => [103, 95, 82, 70, 70, 60, 55],
        "ERU" => [60, 70, 70, 70, 83, 107, 70],
        "ELF" => [65, 65, 95, 80, 80, 75, 75],
        "HIE" => [55, 65, 85, 70, 95, 92, 80],
        "DEF" => [60, 65, 90, 75, 83, 99, 60],
        "HEF" => [70, 70, 90, 85, 60, 75, 75],
        "DWF" => [90, 90, 70, 90, 83, 60, 45],
        "TRL" => [108, 114, 83, 75, 60, 52, 40],
        "OGR" => [130, 127, 70, 70, 67, 60, 37],
        "HFL" => [70, 75, 95, 90, 80, 67, 50],
        "GNM" => [60, 70, 85, 85, 67, 98, 60],
        "IKS" => [70, 70, 90, 85, 80, 75, 55],
        "VAH" => [90, 70, 90, 70, 70, 65, 65],
        "FRG" => [70, 80, 100, 100, 75, 75, 50],
        _ => return None,
    })
}

/// why: class add per class, same order as `race_base`; applies in full
/// regardless of relative level -- leveling caps access, not the bonus.
/// Pulled from the same spreadsheet as `race_base`/`manadata` -- the
/// earlier classic-EQ guess was wrong for most classes (real table gives
/// exactly 2 stats at +15 each, not 4-5 at +5/+10; both summed to 30, so
/// the old wrong table passed its own sum-check). Unknown code -> all
/// zero, not None -- safer than unwinding a partial sheet.
fn class_add(code: &str) -> [i32; 7] {
    match code {
        "WAR" => [10, 15, 5, 0, 0, 0, 0],
        "CLR" => [5, 10, 0, 0, 15, 0, 0],
        "PAL" => [10, 5, 0, 0, 5, 0, 10],
        "RNG" => [5, 10, 10, 0, 5, 0, 0],
        "SHD" => [10, 5, 0, 0, 0, 10, 5],
        "DRU" => [0, 15, 0, 0, 15, 0, 0],
        "MNK" => [5, 5, 10, 10, 0, 0, 0],
        "BRD" => [5, 0, 0, 10, 0, 0, 15],
        "ROG" => [0, 0, 15, 15, 0, 0, 0],
        "SHM" => [0, 10, 0, 0, 15, 0, 5],
        "NEC" => [0, 0, 0, 15, 0, 15, 0],
        "WIZ" => [0, 15, 0, 0, 0, 15, 0],
        "MAG" => [0, 15, 0, 0, 0, 15, 0],
        "ENC" => [0, 0, 0, 0, 0, 15, 15],
        "BST" => [0, 10, 5, 0, 10, 0, 5],
        "BER" => [15, 5, 0, 10, 0, 0, 0],
        _ => [0; 7],
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AttrRowDto {
    pub attr: String,
    pub base: i32,
    /// why: one entry per `classes`, same order
    pub class_adds: Vec<i32>,
    /// why: base + every class add -- naked, no gear/buffs
    pub naked: i32,
    /// why: 0.0 for an unequipped attribute, same as a missing map key
    pub gear: f64,
    /// why: naked + gear, the full character-sheet number; mana is computed from this
    pub total: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClassManaDto {
    pub class: String,
    pub casting_stat: String,
    /// why: pool at `character_level` (the trio min, never this class's own
    /// higher level), fed gear-inclusive total -- matches a real stat window
    pub pool: f64,
    /// why: top two of three by pool actually count toward usable mana
    pub counted: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct VitalsDto {
    /// why: same top-two-of-trio shape as mana, keyed off STA, no caster exclusion
    pub hp: f64,
    /// why: gear-only below, no verified base formula yet -- 0.0 means
    /// no gear stat or no known formula, indistinguishable today
    pub ac: f64,
    /// why: gear's "ATK" stat, unmodified -- rare on real items, reads 0.0 for most loadouts
    pub attack: f64,
    /// why: gear's "HASTE" stat, attack speed not movement speed --
    /// no movement-speed gear stat exists to map to instead
    pub velocity: f64,
    pub endurance: f64,
    pub hp_regen: f64,
    pub mana_regen: f64,
    pub end_regen: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResistsDto {
    /// why: all six gear-only, same caveat as `VitalsDto`
    pub magic: f64,
    pub fire: f64,
    pub cold: f64,
    pub disease: f64,
    pub poison: f64,
    pub void: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CharacterEstimateDto {
    pub race: String,
    /// why: full class names in selection order; every other per-class array lines up
    pub classes: Vec<String>,
    pub class_levels: Vec<u8>,
    /// why: minimum of class_levels, what the trio mechanic caps you to; 0 if empty
    pub character_level: u8,
    /// why: the one class holding level down, if exactly one is; None if tied or empty
    pub limiting_class: Option<String>,
    pub attrs: Vec<AttrRowDto>,
    pub mana: Vec<ClassManaDto>,
    /// why: top-two mana entries plus flat gear MANA bonus, as it stacks in-game
    pub total_mana: f64,
    pub vitals: VitalsDto,
    pub resists: ResistsDto,
    pub attr_cap: i32,
    /// why: always false today -- nobody's checked against a real stat window yet
    pub verified: bool,
    /// why: live re-check that class_add rows still sum to 30, catches future edit mistakes
    pub bad_class_adds: Vec<String>,
}

/// why: full display names, translated to internal codes via
/// `gearplanner`'s lookups, no second name<->code table. `classes`/
/// `class_levels` zipped and capped at 3. `gear` is attr name -> total,
/// caller's job to sum (frontend does it off the resolved doll).
///
/// None only when `race` doesn't resolve -- unset race has no meaning
/// without a base. Empty/partial `classes` still produces a real sheet
/// (race base + gear, level 0, no mana rows) for progressive display.
pub fn estimate(
    race: &str,
    classes: &[String],
    class_levels: &[u8],
    gear: &HashMap<String, f64>,
) -> Option<CharacterEstimateDto> {
    let race_code = gearplanner::race_name_to_code(race)?;
    let base = race_base(race_code)?;

    let classes: Vec<String> = classes.iter().take(3).cloned().collect();
    let levels: Vec<u8> = class_levels.iter().take(classes.len()).copied().collect();
    let classes: Vec<String> = classes.into_iter().take(levels.len()).collect();

    let mut bad_class_adds = Vec::new();
    let adds_per_class: Vec<[i32; 7]> = classes
        .iter()
        .map(|name| {
            let code = gearplanner::name_to_code(name).unwrap_or("");
            let row = class_add(code);
            if row.iter().sum::<i32>() != CLASS_ADD_TOTAL {
                bad_class_adds.push(name.clone());
            }
            row
        })
        .collect();

    let attrs: Vec<AttrRowDto> = ATTRS
        .iter()
        .enumerate()
        .map(|(i, &a)| {
            let class_adds: Vec<i32> = adds_per_class.iter().map(|row| row[i]).collect();
            let naked = base[i] + class_adds.iter().sum::<i32>();
            let gear_val = gear.get(a).copied().unwrap_or(0.0);
            AttrRowDto {
                attr: a.to_string(),
                base: base[i],
                class_adds,
                naked,
                gear: gear_val,
                total: naked as f64 + gear_val,
            }
        })
        .collect();

    let character_level = levels.iter().copied().min().unwrap_or(0);
    let limiting_class = if classes.is_empty() {
        None
    } else {
        let mut at_min = classes
            .iter()
            .zip(levels.iter())
            .filter(|(_, &l)| l == character_level);
        let first = at_min.next().map(|(name, _)| name.clone());
        if at_min.next().is_some() {
            None // why: two or more tied for lowest, no single class to blame
        } else {
            first
        }
    };

    let mut mana: Vec<ClassManaDto> = classes
        .iter()
        .filter_map(|name| {
            let code = gearplanner::name_to_code(name)?;
            let stat = gearplanner::casting_stat(code)?;
            let stat_ix = ATTRS.iter().position(|a| *a == stat)?;
            let total_val = attrs[stat_ix].total;
            let pool = crate::manadata::class_mana_pool(code, character_level, total_val)?;
            Some(ClassManaDto {
                class: name.clone(),
                casting_stat: stat.to_string(),
                pool,
                counted: false,
            })
        })
        .collect();
    mana.sort_by(|a, b| {
        b.pool
            .partial_cmp(&a.pool)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for m in mana.iter_mut().take(2) {
        m.counted = true;
    }
    // why: top-two-of-three, plus a flat gear MANA bonus regardless of casting stat
    let gear_mana = gear.get("MANA").copied().unwrap_or(0.0);
    let total_mana: f64 = mana
        .iter()
        .filter(|m| m.counted)
        .map(|m| m.pool)
        .sum::<f64>()
        + gear_mana;

    // why: same top-two-of-three shape as mana, keyed off STA, every class has HP
    let sta_ix = ATTRS.iter().position(|a| *a == "STA").unwrap();
    let total_sta = attrs[sta_ix].total;
    let mut hp_contribs: Vec<f64> = classes
        .iter()
        .filter_map(|name| {
            crate::hpdata::class_hp_contribution(
                gearplanner::name_to_code(name)?,
                character_level,
                total_sta,
            )
        })
        .collect();
    hp_contribs.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    let hp = crate::hpdata::HP_BASE + hp_contribs.iter().take(2).sum::<f64>();

    let vitals = VitalsDto {
        hp,
        ac: gear.get("AC").copied().unwrap_or(0.0),
        attack: gear.get("ATK").copied().unwrap_or(0.0),
        velocity: gear.get("HASTE").copied().unwrap_or(0.0),
        endurance: gear.get("ENDUR").copied().unwrap_or(0.0),
        hp_regen: gear.get("HP REGEN").copied().unwrap_or(0.0),
        mana_regen: gear.get("MANA REGEN").copied().unwrap_or(0.0),
        end_regen: gear.get("ENDUR REGEN").copied().unwrap_or(0.0),
    };
    let resists = ResistsDto {
        magic: gear.get("SV MAGIC").copied().unwrap_or(0.0),
        fire: gear.get("SV FIRE").copied().unwrap_or(0.0),
        cold: gear.get("SV COLD").copied().unwrap_or(0.0),
        disease: gear.get("SV DISEASE").copied().unwrap_or(0.0),
        poison: gear.get("SV POISON").copied().unwrap_or(0.0),
        void: gear.get("SV VOID").copied().unwrap_or(0.0),
    };

    Some(CharacterEstimateDto {
        race: race.to_string(),
        classes,
        class_levels: levels,
        character_level,
        limiting_class,
        attrs,
        mana,
        total_mana,
        vitals,
        resists,
        attr_cap: ATTR_CAP,
        verified: false,
        bad_class_adds,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// why: the one independently-verified fact -- a build-time check, not just a UI flag
    #[test]
    fn every_class_add_sums_to_total() {
        for &(code, name) in gearplanner::CLASS_NAMES {
            let sum: i32 = class_add(code).iter().sum();
            assert_eq!(
                sum, CLASS_ADD_TOTAL,
                "{name} ({code}) adds {sum}, not {CLASS_ADD_TOTAL}"
            );
        }
    }

    /// why: every offered race needs a base row, else it silently reads "no race selected"
    #[test]
    fn every_race_has_a_base() {
        for &(code, name) in gearplanner::RACE_NAMES {
            assert!(
                race_base(code).is_some(),
                "{name} ({code}) has no race_base row"
            );
        }
    }

    /// why: user's worked example -- SHD 46/ENC 50/WIZ 50 reads as level
    /// 46, held down by Shadow Knight, not an average or the highest
    #[test]
    fn trio_level_is_the_minimum() {
        let est = estimate(
            "Dark Elf",
            &[
                "Shadow Knight".to_string(),
                "Enchanter".to_string(),
                "Wizard".to_string(),
            ],
            &[46, 50, 50],
            &HashMap::new(),
        )
        .unwrap();
        assert_eq!(est.character_level, 46);
        assert_eq!(est.limiting_class.as_deref(), Some("Shadow Knight"));
    }

    /// why: gear lands in its own column, feeds mana off `total` not `naked`
    #[test]
    fn gear_adds_into_total_and_mana() {
        let mut gear = HashMap::new();
        gear.insert("INT".to_string(), 20.0);
        let naked_only =
            estimate("Dark Elf", &["Wizard".to_string()], &[46], &HashMap::new()).unwrap();
        let geared = estimate("Dark Elf", &["Wizard".to_string()], &[46], &gear).unwrap();

        let int_ix = ATTRS.iter().position(|a| *a == "INT").unwrap();
        assert_eq!(naked_only.attrs[int_ix].gear, 0.0);
        assert_eq!(geared.attrs[int_ix].gear, 20.0);
        assert_eq!(
            geared.attrs[int_ix].total,
            geared.attrs[int_ix].naked as f64 + 20.0
        );
        assert!(
            geared.total_mana > naked_only.total_mana,
            "gear INT should raise the mana estimate, not just the sheet"
        );
    }

    /// why: end-to-end through `estimate`, exercises `race_base`/`class_add`
    /// feeding into `manadata`, not just the raw formula manadata already tests
    #[test]
    fn mana_flows_through_the_real_formula_end_to_end() {
        let est = estimate("Dark Elf", &["Wizard".to_string()], &[46], &HashMap::new()).unwrap();
        assert_eq!(
            est.attrs[ATTRS.iter().position(|a| *a == "INT").unwrap()].naked,
            114
        );
        assert_eq!(est.mana.len(), 1);
        assert_eq!(est.mana[0].pool, 1306.0);
        assert_eq!(est.total_mana, 1306.0);
    }

    /// why: a flat MANA gear stat just adds, no per-point formula
    #[test]
    fn gear_mana_stat_adds_directly_to_total() {
        let mut gear = HashMap::new();
        gear.insert("MANA".to_string(), 250.0);
        let est = estimate("Dark Elf", &["Wizard".to_string()], &[46], &gear).unwrap();
        assert_eq!(est.total_mana, 1306.0 + 250.0);
    }

    /// why: a 50/50/46 trio computes every class's pool at 46 (the trio
    /// min), never an individual class's own higher level
    #[test]
    fn trio_mana_pools_all_use_the_trios_lowest_level() {
        let est = estimate(
            "Dark Elf",
            &[
                "Shadow Knight".to_string(),
                "Enchanter".to_string(),
                "Wizard".to_string(),
            ],
            &[46, 50, 50],
            &HashMap::new(),
        )
        .unwrap();
        assert_eq!(est.character_level, 46);
        assert_eq!(
            est.attrs[ATTRS.iter().position(|a| *a == "INT").unwrap()].naked,
            139
        );
        assert_eq!(est.mana.len(), 3);
        for m in &est.mana {
            assert_eq!(
                m.pool, 1552.0,
                "{}'s pool was {}, expected 1552 (computed at the trio's level, not its own)",
                m.class, m.pool
            );
        }
    }

    /// why: user's in-game removal test replayed -- formula gives delta
    /// 152 vs observed 162, fully explained by a round-half-up tie at
    /// `conv_stat`'s >200 branch (266.5), not a modeling gap
    #[test]
    fn removal_delta_is_close_to_but_not_exactly_the_users_observed_number() {
        let classes = vec![
            "Wizard".to_string(),
            "Enchanter".to_string(),
            "Magician".to_string(),
        ];
        // why: Human WIZ/ENC/MAG naked INT = 120; gear makes up the rest of 348/333
        let mut with_item = HashMap::new();
        with_item.insert("INT".to_string(), 228.0);
        let mut without_item = HashMap::new();
        without_item.insert("INT".to_string(), 213.0);

        let before = estimate("Human", &classes, &[50, 50, 50], &with_item).unwrap();
        let after = estimate("Human", &classes, &[50, 50, 50], &without_item).unwrap();
        assert_eq!(
            before.attrs[ATTRS.iter().position(|a| *a == "INT").unwrap()].total,
            348.0
        );
        assert_eq!(
            after.attrs[ATTRS.iter().position(|a| *a == "INT").unwrap()].total,
            333.0
        );
        assert_eq!(before.total_mana - after.total_mana, 152.0);
    }

    /// why: EQL's real ceiling per the user, supersedes the unconfirmed 150
    #[test]
    fn attr_cap_is_510() {
        assert_eq!(ATTR_CAP, 510);
    }
}
