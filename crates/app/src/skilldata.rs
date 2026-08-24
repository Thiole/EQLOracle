//! why: skill -> class lookup, same pattern as `classdata.rs`/`stancedata.rs`
//!
//! Only skills purely class-gated count as evidence. Tracking included
//! (Bard/Druid/Ranger only). Forage deliberately excluded -- Iksar/Wood
//! Elf get it from race regardless of class, would false-positive.

use std::collections::HashMap;
use std::sync::OnceLock;

const SKILL_DATA_JSON: &str = include_str!("../../../packs/skill_classes.json");

static SKILL_DATA: OnceLock<HashMap<String, Vec<String>>> = OnceLock::new();

/// why: empty means unknown skill, not zero eligible classes
pub fn classes_for(skill: &str) -> &'static [String] {
    let map = SKILL_DATA.get_or_init(|| {
        serde_json::from_str(SKILL_DATA_JSON)
            .unwrap_or_else(|e| panic!("packs/skill_classes.json failed to parse: {e}"))
    });
    map.get(skill).map(|v| v.as_slice()).unwrap_or(&[])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracking_is_bard_druid_ranger() {
        let classes = classes_for("Tracking");
        assert_eq!(classes.len(), 3);
        for c in ["Bard", "Druid", "Ranger"] {
            assert!(classes.contains(&c.to_string()), "{classes:?} missing {c}");
        }
    }

    #[test]
    fn forage_is_deliberately_not_here() {
        assert!(classes_for("Forage").is_empty());
    }

    #[test]
    fn an_unrecognized_skill_is_unknown_not_ineligible() {
        assert!(classes_for("Not A Real Skill").is_empty());
    }
}
