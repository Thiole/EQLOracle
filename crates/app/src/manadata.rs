//! The real, authoritative per-class mana formula -- not reverse-engineered
//! from play data (every earlier attempt at that in this app's history was
//! wrong in a new way each time), pulled directly from a spreadsheet the
//! user found and shared that reconstructs it from `EQEmu`-derived client
//! formulas. Verified against nine real measurements the user took in-game
//! (five naked stat-window readings across five different levels, four
//! item-removal deltas at level 50): four of the five absolute readings
//! matched exactly, the fifth was within 0.3%, and every delta matched.
//!
//! The real formula has two parts:
//!
//! 1. [`conv_stat`] -- the casting stat (INT or WIS) isn't used raw. Points
//!    up to 100 count fully; 101-200 have diminishing returns; above 200 a
//!    second diminishing-returns pass applies on top of the first. This
//!    single piecewise formula is shared by every class -- confirmed
//!    directly from the source spreadsheet's own `Calc_Backend` sheet, not
//!    inferred.
//! 2. A per-class, per-level `(base, fac)` pair (`packs/mana_table.json`,
//!    extracted from that same spreadsheet's `base_data` sheet): one
//!    class's own mana pool at a given level is `base + fac *
//!    conv_stat(casting_stat)`, floored. `character::estimate` sums the
//!    *top two* of the three active classes' own pools -- see that
//!    module's doc for why, confirmed directly by the source spreadsheet's
//!    own "Growth Charts" tab note: "the game will calculate all 3 classes
//!    individually and sum the two highest/best results."
//!
//! Only classes with a real mana pool are in the table (12 of 16 --
//! WAR/MNK/ROG/BER have none, `mana_row` returns `None` for them the same
//! as for a level outside 1-50, this game's real level cap).

use serde::Deserialize;
use std::collections::HashMap;
use std::sync::OnceLock;

const MANA_TABLE_JSON: &str = include_str!("../../../packs/mana_table.json");

#[derive(Debug, Clone, Deserialize)]
struct ManaRow {
    level: u8,
    base: f64,
    fac: f64,
}

static MANA_TABLE: OnceLock<HashMap<String, Vec<ManaRow>>> = OnceLock::new();

fn table() -> &'static HashMap<String, Vec<ManaRow>> {
    MANA_TABLE.get_or_init(|| {
        // A malformed embedded file is a build-time data bug, loud and
        // immediate -- same stance every other `include_str!`-baked pack
        // in this app takes (see e.g. `itemdata::items`'s doc).
        serde_json::from_str(MANA_TABLE_JSON)
            .unwrap_or_else(|e| panic!("packs/mana_table.json failed to parse: {e}"))
    })
}

/// `class_code`'s own `(base, fac)` at `level` -- `None` for a class with
/// no mana pool (WAR/MNK/ROG/BER aren't in the table at all) or a level
/// outside 1-50 (this game's own real level cap, and the table's exact
/// range). Rows are stored level-ordered starting at 1, so this is a
/// direct index, not a scan.
fn mana_row(class_code: &str, level: u8) -> Option<(f64, f64)> {
    let rows = table().get(class_code)?;
    let row = rows.get(usize::from(level).checked_sub(1)?)?;
    debug_assert_eq!(
        row.level, level,
        "packs/mana_table.json rows must be level-ordered from 1 with no gaps"
    );
    Some((row.base, row.fac))
}

/// EQ's own "converted stat" formula for mana -- confirmed directly from
/// the source spreadsheet's `Calc_Backend` sheet (`Converted INT`/
/// `Converted WIS` cells), not inferred: full value up to 100, then
/// diminishing returns 101-200, then a second diminishing-returns pass
/// stacked on top above 200. Shared by every casting class -- there's only
/// one of these, not a per-class variant.
pub(crate) fn conv_stat(stat: f64) -> f64 {
    if stat <= 0.0 {
        0.0
    } else if stat <= 100.0 {
        stat
    } else if stat <= 200.0 {
        ((5.0 * stat - 300.0) / 2.0).round()
    } else {
        let inner = ((stat + 200.0) / 2.0).round();
        ((5.0 * inner - 300.0) / 2.0).round()
    }
}

/// One class's own mana pool at `level`, given `stat` (its own casting
/// stat's *total* value, gear included -- see `conv_stat` for how it's
/// actually used). `None` for a class with no mana pool or a level
/// outside 1-50 -- see `mana_row`'s own doc.
pub(crate) fn class_mana_pool(class_code: &str, level: u8, stat: f64) -> Option<f64> {
    let (base, fac) = mana_row(class_code, level)?;
    // The source formula wraps this in Excel's INT(), which truncates
    // toward zero -- `.floor()` is exactly that for the always-positive
    // values a real pool produces.
    Some((base + fac * conv_stat(stat)).floor())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The user's own five real naked (no gear) measurements, replayed
    /// exactly -- see this module's own doc for the full context. Four of
    /// five are exact; the Shadow Knight trio is within 0.3%; the doc
    /// explains why.
    #[test]
    fn matches_real_naked_measurements() {
        let cases: &[(&str, u8, f64, &[&str], f64)] = &[
            ("A", 11, 105.0, &["DRU", "WIZ", "ENC"], 514.0),
            ("B", 50, 120.0, &["MAG", "WIZ", "ENC"], 3150.0),
            ("C", 28, 120.0, &["WIZ", "NEC", "ENC"], 1470.0),
            ("D", 46, 115.0, &["SHD", "WIZ", "ENC"], 2628.0),
            ("E", 10, 105.0, &["WIZ", "ENC", "CLR"], 468.0),
        ];
        for &(name, level, stat, classes, real) in cases {
            let mut pools: Vec<f64> = classes
                .iter()
                .map(|c| class_mana_pool(c, level, stat).unwrap())
                .collect();
            pools.sort_by(|a, b| b.partial_cmp(a).unwrap());
            let total: f64 = pools[0] + pools[1];
            let diff_pct = (total - real).abs() / real * 100.0;
            assert!(
                diff_pct < 1.0,
                "case {name}: predicted {total}, real {real} ({diff_pct:.2}% off)"
            );
        }
    }

    /// `conv_stat`'s own boundary behavior, confirmed against the source
    /// spreadsheet's own "Level 50 Mana/Stat" reference table (not just
    /// re-derived from the formula that also produced this function).
    #[test]
    fn conv_stat_matches_the_reference_table() {
        assert_eq!(conv_stat(60.0), 60.0);
        assert_eq!(conv_stat(100.0), 100.0);
        assert_eq!(conv_stat(101.0), 103.0);
        assert_eq!(conv_stat(105.0), 113.0);
        assert_eq!(conv_stat(108.0), 120.0);
    }
}
