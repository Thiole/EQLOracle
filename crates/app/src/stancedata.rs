//! why: stance -> class lookup, same pattern `classdata.rs` uses for spells
//!
//! 9 real stances confirmed against eqlwiki, "Berserker" unambiguous
//! (one class). Only ever evidence for "You" -- log reports no one else's.

use std::collections::HashMap;
use std::sync::OnceLock;

const STANCE_DATA_JSON: &str = include_str!("../../../packs/stance_classes.json");

static STANCE_DATA: OnceLock<HashMap<String, Vec<String>>> = OnceLock::new();

/// why: empty means unknown, not zero eligible; case-insensitive vs log
pub fn classes_for(stance: &str) -> &'static [String] {
    let map = STANCE_DATA.get_or_init(|| {
        serde_json::from_str(STANCE_DATA_JSON)
            .unwrap_or_else(|e| panic!("packs/stance_classes.json failed to parse: {e}"))
    });
    map.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(stance))
        .map(|(_, v)| v.as_slice())
        .unwrap_or(&[])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn berserker_is_unambiguous() {
        assert_eq!(classes_for("Berserker"), &["Berserker".to_string()]);
    }

    #[test]
    fn evasive_does_not_include_druid() {
        // why: easy mix-up, Channeler includes Druid, Evasive doesn't
        let classes = classes_for("Evasive");
        assert!(classes.contains(&"Ranger".to_string()));
        assert!(classes.contains(&"Bard".to_string()));
        assert!(!classes.contains(&"Druid".to_string()));
    }

    #[test]
    fn an_unrecognized_stance_is_unknown_not_ineligible() {
        assert!(classes_for("Not A Real Stance").is_empty());
    }

    #[test]
    fn lookup_is_case_insensitive_matching_the_log_s_own_lowercase_form() {
        assert_eq!(classes_for("evasive"), classes_for("Evasive"));
        assert_eq!(classes_for("mage hunter"), classes_for("Mage Hunter"));
    }
}
