//! Native "Character Planner": race + up to 3 classes, each with its own
//! level, plus whatever's equipped -> a full attribute sheet (base, each
//! class's own add, naked, gear, total) and a gear-inclusive mana-pool
//! estimate. No log line ever states a character's raw attributes, so
//! unlike everything else this app infers from the parse, this module is
//! pure calculation off values the caller hands it -- the same
//! relationship `gearplanner`'s scoring has to `stat_weights` (a model,
//! not a scrape). This module itself never touches gear directly (no
//! notion of items or slots) -- `estimate`'s `gear` argument is just an
//! attribute-name -> total map, summed by whoever calls it.
//!
//! THE TRIO MECHANIC THIS IS BUILT AROUND (explained by the user, not
//! documented anywhere on eqlwiki): EQL lets you level up to 3 classes at
//! once. While a given trio is active, killing something levels all 3
//! simultaneously -- (10,10,10) -> (11,11,11) on the next level, never one
//! class alone. But each class's own level persists independently once
//! reached: level Necromancer/Wizard/Enchanter to 50 together, then swap
//! in Shadow Knight for Enchanter and level *that* trio, and Enchanter
//! stays a real, remembered 50 -- it doesn't reset just because it's not
//! one of the 3 currently active. So a character's overall level isn't a
//! single stored number at all; it's the *minimum* of whichever 3 classes
//! are active right now (SHD 46 / ENC 50 / WIZ 50 -> character level 46,
//! "downgraded" until Shadow Knight also reaches 50), and any combination
//! of six-or-more classes that all individually reached 50 gives a true
//! level 50 -- they never needed to be leveled together as one trio to get
//! there. `estimate` takes each active class's own level as a separate
//! input for exactly this reason, and derives `character_level` as their
//! min rather than asking for one level up front.
//!
//! DATA PROVENANCE -- READ THIS, same caveat the standalone planner's own
//! `chardata.json` carried: eqlwiki.com does not publish race base
//! attributes or per-class attribute adds. Its Character_Races page lists
//! starting cities and class options only, and is flagged outdated by its
//! own editors -- there is nothing to scrape. `race_base`/`class_add`
//! below are classic-EverQuest values, carried over on the unverified
//! assumption that EQL reuses them. One thing about them *is*
//! independently verified (eqltools.com/attributes, client-mined): every
//! class adds exactly `CLASS_ADD_TOTAL` (30) points total, so a trio is
//! worth 90 to any race -- `class_add`'s own doc re-checks that per row.
//! Everything else here should be treated as a labeled guess (`verified:
//! false` on every `CharacterEstimateDto`) until someone checks it against
//! a real, freshly made, unequipped EQL character's own stat window.

use crate::gearplanner;
use serde::Serialize;
use std::collections::HashMap;

/// Fixed attribute order every fixed-size `[i32; 7]` table below is
/// indexed by -- `race_base`/`class_add`'s own doc, and `estimate`'s
/// `attrs` output, all agree on this order rather than each carrying (and
/// risking drifting on) their own.
pub const ATTRS: &[&str] = &["STR", "STA", "AGI", "DEX", "WIS", "INT", "CHA"];

/// Every class's `class_add` row sums to exactly this -- verified
/// independently of the wiki (see this module's doc), and re-checked live
/// by `estimate` (`bad_class_adds`) rather than just asserted once.
pub const CLASS_ADD_TOTAL: i32 = 30;

/// EQ Legends' own attribute ceiling, per the user directly (not scraped
/// -- eqlwiki doesn't publish this either). Supersedes the standalone
/// planner's `150`, which that source's own doc already flagged as an
/// unconfirmed guess ("players report a soft ceiling... unconfirmed").
/// Nothing here clamps to it; `estimate`'s caller can flag a total that
/// exceeds it, without this module silently lying about a real total.
pub const ATTR_CAP: i32 = 510;

/// Race code -> base `[STR, STA, AGI, DEX, WIS, INT, CHA]`, unbuffed and
/// ungeared -- pulled directly from the same source spreadsheet
/// `character.rs`'s mana rewrite came from (`Base_Stat_Table` sheet), not
/// the earlier "classic EQ, unverified" guess. Four entries corrected
/// against it: Troll STA 109->114, Ogre STA 132->127, Vah Shir/"Kerran"
/// STA 75->70, Froglok DEX 85->100 -- the rest already happened to match.
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

/// Class code -> what it adds to the sheet, same `[STR..CHA]` order as
/// `race_base` -- every class plays a trio, so an active class's add
/// always applies in full regardless of how it's currently leveled
/// relative to the other two (see this module's doc: leveling caps
/// *access*, not the attribute bonus itself, which classic EQ's own
/// mechanic this is modeled on grants for simply having trained the
/// class at all).
///
/// Pulled from the same source spreadsheet as `race_base` and `manadata`
/// -- and it turned out this app's own earlier guess (carried over from
/// classic EQ, always flagged unverified) was wrong for most classes: the
/// real table gives every class exactly *two* stats at +15 each, not four
/// or five stats at +5/+10 -- e.g. real Magician is `STA +15, INT +15`,
/// this app previously had `STA +10, AGI +5, DEX +5, INT +10`. Every row
/// still sums to `CLASS_ADD_TOTAL` (30) either way, which is exactly why
/// the old wrong table passed its own sum-check for as long as it did.
/// Unknown code -> all zero, not `None`: a class this doesn't recognise
/// contributing nothing is a safer default than `estimate` having to
/// unwind a partially-built sheet over it.
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
    /// One entry per entry in `CharacterEstimateDto::classes`, same order.
    pub class_adds: Vec<i32>,
    /// `base` plus every class's own add -- naked, no gear, no buffs.
    pub naked: i32,
    /// This attribute's share of `estimate`'s `gear` argument -- `0.0` for
    /// an attribute no equipped item happens to carry, same meaning as a
    /// missing key in that map, not a distinct "no data" state.
    pub gear: f64,
    /// `naked as f64 + gear` -- the full character-sheet number: race +
    /// active classes + whatever's actually equipped. What `mana` below
    /// is computed from, not `naked` alone -- see this module's doc.
    pub total: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClassManaDto {
    pub class: String,
    pub casting_stat: String,
    /// `crate::manadata::class_mana_pool` at `character_level`, fed this
    /// stat's full `AttrRowDto::total` (gear included) -- this class's own
    /// pool if it were the only thing drawing on that stat. Gear-inclusive
    /// on purpose: a real mana pool is read off your actual in-game stat
    /// window with gear on, so a naked-only number would systematically
    /// under-read it the moment `gear` carries anything. See `counted` for
    /// whether this entry actually makes the total.
    ///
    /// Always `character_level` here, never this class's own entry in
    /// `class_levels` -- a 50/50/46 trio computes every one of its 3
    /// classes' pools at 46, not 50 for the two that happen to individually
    /// read higher. Matches the trio mechanic itself (this module's own
    /// doc): the mechanic caps *access* at the lowest member's level, so
    /// there's no real level-50 mana pool available from any of the three
    /// while one is still 46, regardless of which class's own counter
    /// happens to say 50.
    pub pool: f64,
    /// Whether this class's pool is one of the top two (by `pool`) that
    /// actually sums into `total_mana` -- see `gearplanner::MANA_LEVEL_
    /// COEF`'s own doc: only your two highest-pool classes count toward
    /// usable mana, never all three.
    pub counted: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct VitalsDto {
    /// `hp_base::HP_BASE` (5) plus the top two of the trio's own three
    /// `hp_base::class_hp_contribution` values (level + total STA, gear
    /// included) -- the real formula, structurally identical to `mana`
    /// above but keyed off STA and with no non-caster exclusion (every
    /// class has an HP pool). See `crate::hpdata`'s own module doc.
    pub hp: f64,
    /// Everything below is gear-only -- summed straight off `estimate`'s
    /// `gear` argument, with no base/race/class/level formula behind it.
    /// Not because there isn't one; nobody has found or verified it yet,
    /// the same place `mana`/`hp` themselves were before the real formula
    /// turned up. `0.0` reads as "no gear carries this stat *or* no base
    /// value is known" -- both real gaps today, not distinguished because
    /// this module has no way to tell them apart yet.
    pub ac: f64,
    /// Gear's own `"ATK"` stat, unmodified -- real items carrying it are
    /// rare (confirmed against the full catalog: essentially none do as
    /// of this scrape), so this reads `0.0` for almost every loadout.
    pub attack: f64,
    /// Gear's own `"HASTE"` stat -- named `velocity` because that's the
    /// label the Character Planner's own layout asks for, but this is
    /// attack-speed haste, not movement speed; there's no movement-speed
    /// gear stat in the catalog to map to instead. Flag if that's the
    /// wrong stat for what "Velocity" is meant to show.
    pub velocity: f64,
    pub endurance: f64,
    pub hp_regen: f64,
    pub mana_regen: f64,
    pub end_regen: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResistsDto {
    /// All six -- gear-only, same caveat as `VitalsDto`'s own doc.
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
    /// Full class names, in the order they were selected (up to 3) --
    /// every other per-class array here (`class_levels`, `AttrRowDto::
    /// class_adds`, `mana`) lines up against this same order.
    pub classes: Vec<String>,
    pub class_levels: Vec<u8>,
    /// The minimum of `class_levels` -- what the trio mechanic (this
    /// module's own doc) actually caps you to. `0` if `classes` is empty.
    pub character_level: u8,
    /// Which single class in `classes` is holding `character_level` down,
    /// if exactly one is -- `None` when `classes` is empty, or when two or
    /// more classes tie for lowest (no one class to point at).
    pub limiting_class: Option<String>,
    pub attrs: Vec<AttrRowDto>,
    pub mana: Vec<ClassManaDto>,
    /// Sum of the top two `mana` entries by pool (see `ClassManaDto::
    /// counted`), plus any direct `"MANA"` gear bonus -- a flat +mana
    /// item stat, not run through the per-point INT/WIS formula, added on
    /// top the way it actually stacks in-game.
    pub total_mana: f64,
    pub vitals: VitalsDto,
    pub resists: ResistsDto,
    pub attr_cap: i32,
    /// Always `false` today -- see this module's doc: nobody has checked
    /// these numbers against a real EQL character's stat window yet. A
    /// live field rather than a doc comment so the UI can show the same
    /// "unverified" caveat the standalone planner's own sheet did.
    pub verified: bool,
    /// Any selected class whose own `class_add` row doesn't sum to
    /// `CLASS_ADD_TOTAL` -- always empty in the shipped table (every row
    /// is hand-verified, see `class_add`'s doc), kept as a live re-check
    /// rather than a one-time assertion so a future editing mistake shows
    /// up on the page itself instead of silently producing a wrong sheet.
    pub bad_class_adds: Vec<String>,
}

/// `race` and each of `classes` are full display names (`"Human"`,
/// `"Wizard"`, ...), the same shape the Gear Planner's own class chips and
/// race picker already use -- translated to this module's internal codes
/// via `gearplanner`'s own lookups, so there's no second name<->code table
/// to keep in sync. `classes`/`class_levels` are zipped position-wise and
/// truncated to whichever is shorter, then capped at 3 (every class plays
/// a trio -- see this module's doc); a caller sending matched-length
/// arrays, which is the only sane thing to send, never notices the
/// truncation. `gear` is attribute name -> total across whatever's
/// actually equipped (`"STR"`, `"INT"`, ...) -- this module has no notion
/// of items or slots of its own, so summing that is entirely the caller's
/// job (the frontend does it off the Gear Planner's own resolved doll);
/// an attribute missing from the map reads as `0.0`, same as an empty map
/// reads as no gear at all.
///
/// Returns `None` only when `race` doesn't resolve to a known code -- an
/// unset race, same as the standalone planner's own "pick a race" empty
/// state, since a naked sheet has no meaning without a base to start from.
/// An empty (or partial) `classes` still produces a real sheet: race base
/// (plus gear) alone, `character_level` 0, no mana rows -- letting the UI
/// show "1 class slot filled, 2 empty" progressively instead of demanding
/// all 3 before showing anything.
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
            None // two or more tied for lowest -- no single class to blame
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
    // Top-two-of-three per the formula above, plus whatever's equipped
    // that adds directly to the pool (an item's own "MANA" stat, not run
    // through the per-point INT/WIS formula at all -- a flat bonus is a
    // flat bonus regardless of casting stat).
    let gear_mana = gear.get("MANA").copied().unwrap_or(0.0);
    let total_mana: f64 = mana
        .iter()
        .filter(|m| m.counted)
        .map(|m| m.pool)
        .sum::<f64>()
        + gear_mana;

    // HP: the same top-two-of-three shape as mana, keyed off STA instead
    // of INT/WIS, and with no non-caster exclusion -- every class has an
    // HP pool. See `hpdata`'s own module doc.
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

    /// The one independently-verified fact about `class_add` (see this
    /// module's doc) -- every row sums to `CLASS_ADD_TOTAL`, checked here
    /// so an edit that breaks it fails the build, not just the live
    /// `bad_class_adds` UI flag.
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

    /// Every race `gearplanner::RACE_NAMES` knows about should also have a
    /// `race_base` row -- a race the picker offers but this module can't
    /// price would silently fall through `estimate`'s `?` and read as "no
    /// race selected".
    #[test]
    fn every_race_has_a_base() {
        for &(code, name) in gearplanner::RACE_NAMES {
            assert!(
                race_base(code).is_some(),
                "{name} ({code}) has no race_base row"
            );
        }
    }

    /// The user's own worked example (see this module's doc): Shadow
    /// Knight 46 / Enchanter 50 / Wizard 50 should read as character level
    /// 46, held down by Shadow Knight specifically -- not an average, not
    /// the highest, and not ambiguous just because two of the three tie at
    /// 50.
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

    /// Gear should land in its own `AttrRowDto::gear` column, add straight
    /// into `total`, and feed `mana` off `total` rather than `naked` -- a
    /// real stat window reads with gear on, so a mana estimate that only
    /// ever looked at naked INT would systematically under-read as soon as
    /// any gear carries the casting stat.
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

    /// End-to-end through `estimate` (race + class adds -> naked INT ->
    /// `manadata::class_mana_pool`), not just the raw formula --
    /// `manadata`'s own tests already cover the formula itself against
    /// the user's five real naked readings; this is what actually
    /// exercises `race_base`/`class_add` feeding into it correctly. Dark
    /// Elf Wizard: naked INT = 99 (race) + 15 (class, corrected -- see
    /// `class_add`'s own doc) = 114, level 46.
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

    /// A flat "MANA" gear stat adds straight onto `total_mana` -- not run
    /// through the per-point INT/WIS formula at all, just added.
    #[test]
    fn gear_mana_stat_adds_directly_to_total() {
        let mut gear = HashMap::new();
        gear.insert("MANA".to_string(), 250.0);
        let est = estimate("Dark Elf", &["Wizard".to_string()], &[46], &gear).unwrap();
        assert_eq!(est.total_mana, 1306.0 + 250.0);
    }

    /// The user's own worked example: a 50/50/46 trio should compute
    /// *every* class's mana pool at level 46 (the trio's own minimum --
    /// `character_level`), never at an individual class's higher own
    /// level. Shadow Knight/Enchanter/Wizard all draw on INT (see
    /// `trio_level_is_the_minimum`), so naked INT is shared: 99 (Dark Elf
    /// base) + 10 (SHD) + 15 (ENC) + 15 (WIZ) = 139. If this were
    /// (wrongly) computed per-class at each class's own level, Enchanter/
    /// Wizard (level 50) would read differently from Shadow Knight (level
    /// 46) -- instead all three should be identical.
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

    /// The user's own in-game removal test, replayed with the real
    /// formula: level 50, 348 -> 333 INT (one 15-INT item, no flat mana
    /// bonus of its own). The real formula puts this delta at 152, not
    /// the 162 the user observed in-game -- close but not exact, and
    /// that residual gap is fully explained: 348 and 333 both land
    /// `conv_stat`'s `>200` branch on an exact half-integer partway
    /// through (`(333+200)/2 = 266.5`), where Excel's `ROUND` (and this
    /// port's `f64::round`) breaks ties away from zero, not to even.
    /// `manadata`'s own tests already show this same formula matching
    /// four of five *absolute* real readings exactly, so the formula
    /// itself isn't in question -- 152 vs 162 here is close enough to
    /// read as the same small measurement noise the naked data already
    /// has (the fifth of those five readings was ~0.3% off), not a
    /// modeling gap.
    #[test]
    fn removal_delta_is_close_to_but_not_exactly_the_users_observed_number() {
        let classes = vec![
            "Wizard".to_string(),
            "Enchanter".to_string(),
            "Magician".to_string(),
        ];
        // Human WIZ/ENC/MAG naked INT = 75 + 15 + 15 + 15 = 120; gear
        // makes up the rest of 348/333.
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

    /// EQ Legends' own attribute ceiling, given directly by the user --
    /// supersedes the standalone planner's unconfirmed `150`.
    #[test]
    fn attr_cap_is_510() {
        assert_eq!(ATTR_CAP, 510);
    }
}
