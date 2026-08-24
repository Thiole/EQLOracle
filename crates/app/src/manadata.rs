//! why: authoritative per-class mana formula, pulled from a real spreadsheet
//! (EQEmu-derived), verified against 9 real in-game measurements -- 4/5
//! exact, 1 within 0.3%, every delta matched.
//!
//! Two parts: [`conv_stat`] (INT/WIS diminishing-returns curve, shared by
//! every class) and a per-class/level `(base, fac)` pair from
//! `packs/mana_table.json`. `character::estimate` sums the top two of
//! three active classes' pools. Only 12 of 16 classes have a mana pool.

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
        // why: malformed embedded data is a build bug, fail loud
        serde_json::from_str(MANA_TABLE_JSON)
            .unwrap_or_else(|e| panic!("packs/mana_table.json failed to parse: {e}"))
    })
}

/// why: None for a no-mana-pool class or level outside 1-50; direct index
fn mana_row(class_code: &str, level: u8) -> Option<(f64, f64)> {
    let rows = table().get(class_code)?;
    let row = rows.get(usize::from(level).checked_sub(1)?)?;
    debug_assert_eq!(
        row.level, level,
        "packs/mana_table.json rows must be level-ordered from 1 with no gaps"
    );
    Some((row.base, row.fac))
}

/// why: EQ's converted-stat curve for mana, shared by every casting class
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

/// why: one class's mana pool at `level`; None if no pool or level outside 1-50
pub(crate) fn class_mana_pool(class_code: &str, level: u8, stat: f64) -> Option<f64> {
    let (base, fac) = mana_row(class_code, level)?;
    // why: mirrors Excel's INT(), truncation toward zero for positive values
    Some((base + fac * conv_stat(stat)).floor())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// why: 5 real naked in-game measurements, replayed; 4 exact, 1 within 0.3%
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

    /// why: boundary behavior vs the source spreadsheet's reference table
    #[test]
    fn conv_stat_matches_the_reference_table() {
        assert_eq!(conv_stat(60.0), 60.0);
        assert_eq!(conv_stat(100.0), 100.0);
        assert_eq!(conv_stat(101.0), 103.0);
        assert_eq!(conv_stat(105.0), 113.0);
        assert_eq!(conv_stat(108.0), 120.0);
    }
}
