//! The real per-class HP formula -- pulled from the same source spreadsheet
//! as `crate::manadata`'s mana formula (that module's own doc has the full
//! provenance story). Structurally the same shape as mana -- a per-class,
//! per-level `(base, fac)` pair, summed over the *top two* of the active
//! trio -- but keyed off STA instead of INT/WIS, every class has an HP
//! pool (no WAR/MNK/ROG/BER-style exclusion the way mana has), and there's
//! a flat `+5` added once at the very end rather than folded into either
//! class's own contribution -- confirmed directly from the source
//! spreadsheet's own `Char_Builder` sheet (`=5+LARGE(...,1)+LARGE(...,2)`).

use serde::Deserialize;
use std::collections::HashMap;
use std::sync::OnceLock;

const HP_TABLE_JSON: &str = include_str!("../../../packs/hp_table.json");

/// Flat HP every character gets regardless of class or stats -- the
/// `Char_Builder` sheet's own `5 + ...` on top of the summed class
/// contributions.
pub(crate) const HP_BASE: f64 = 5.0;

#[derive(Debug, Clone, Deserialize)]
struct HpRow {
    level: u8,
    hp: f64,
    hp_fac: f64,
}

static HP_TABLE: OnceLock<HashMap<String, Vec<HpRow>>> = OnceLock::new();

fn table() -> &'static HashMap<String, Vec<HpRow>> {
    HP_TABLE.get_or_init(|| {
        serde_json::from_str(HP_TABLE_JSON)
            .unwrap_or_else(|e| panic!("packs/hp_table.json failed to parse: {e}"))
    })
}

fn hp_row(class_code: &str, level: u8) -> Option<(f64, f64)> {
    let rows = table().get(class_code)?;
    let row = rows.get(usize::from(level).checked_sub(1)?)?;
    debug_assert_eq!(
        row.level, level,
        "packs/hp_table.json rows must be level-ordered from 1 with no gaps"
    );
    Some((row.hp, row.hp_fac))
}

/// EQ's own "adjusted" STA for HP purposes -- confirmed directly from the
/// source spreadsheet's `Calc_Backend` sheet (`Adjusted STA (for HP)`
/// cell): full value up to 255, then diminishing returns above it. A
/// *different* threshold and shape from `manadata::conv_stat`'s own
/// INT/WIS adjustment -- HP and mana clearly aren't the same formula
/// reused, so this isn't factored to share code with that one.
pub(crate) fn adjusted_sta(sta: f64) -> f64 {
    if sta > 255.0 {
        ((sta - 255.0) / 2.0).round() + 255.0
    } else {
        sta
    }
}

/// One class's own HP contribution at `level`, given `sta` (its total
/// value, gear included). `None` for a level outside 1-50.
pub(crate) fn class_hp_contribution(class_code: &str, level: u8, sta: f64) -> Option<f64> {
    let (base, fac) = hp_row(class_code, level)?;
    Some((base + fac * adjusted_sta(sta)).floor())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adjusted_sta_matches_the_source_formula() {
        assert_eq!(adjusted_sta(200.0), 200.0);
        assert_eq!(adjusted_sta(255.0), 255.0);
        assert_eq!(adjusted_sta(265.0), 260.0); // 255 + round((265-255)/2) = 255+5
        assert_eq!(adjusted_sta(300.0), 278.0); // 255 + round((300-255)/2) = 255 + round(22.5) = 255+23
    }

    /// The source spreadsheet's own worked example (`Char_Builder` sheet):
    /// Iksar Magician/Warrior/Rogue, total STA 100 -> HP 621 at level 10,
    /// 3705 at level 50.
    #[test]
    fn matches_the_source_spreadsheets_worked_example() {
        for &(level, expected) in &[(10u8, 621.0), (50u8, 3705.0)] {
            let mut contribs: Vec<f64> = ["MAG", "WAR", "ROG"]
                .iter()
                .map(|c| class_hp_contribution(c, level, 100.0).unwrap())
                .collect();
            contribs.sort_by(|a, b| b.partial_cmp(a).unwrap());
            let hp = HP_BASE + contribs[0] + contribs[1];
            assert_eq!(hp, expected, "level {level}");
        }
    }

    #[test]
    fn every_class_has_an_hp_row_at_level_1_and_50() {
        for code in [
            "WAR", "CLR", "PAL", "RNG", "SHD", "DRU", "MNK", "BRD", "ROG", "SHM", "NEC", "WIZ",
            "MAG", "ENC", "BST", "BER",
        ] {
            assert!(
                class_hp_contribution(code, 1, 75.0).is_some(),
                "{code} missing level 1"
            );
            assert!(
                class_hp_contribution(code, 50, 75.0).is_some(),
                "{code} missing level 50"
            );
        }
    }
}
