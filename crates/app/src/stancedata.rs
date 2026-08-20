//! Wires the scraped stance -> class lookup (`packs/stance_classes.json`)
//! into the live app, same pattern `classdata.rs` uses for spells.
//!
//! Confirmed against eqlwiki.com's "Stances & Invocations" page (fetched
//! 2026-08-19), not guessed: 9 real stances, each with its own class
//! list -- "Berserker" is unambiguous on its own (one class only), the
//! rest range from 3 to 9 classes. Log-line shape ("You assume a/an X
//! stance.") confirmed against real characters' own logs, which is also
//! how the specific stance *names* actually used (Balanced, Channeler,
//! Defensive, Evasive, Offensive) were found -- the other four listed
//! here (Berserker, Mage Hunter, Ranged, Striker) are real per the wiki
//! but unconfirmed in a real log yet; kept anyway since the parser rule
//! matches the stance name generically, not a fixed list, so there's
//! nothing extra to wire up if one of these shows up later.
//!
//! Only ever fed evidence for "You" -- the log doesn't report anyone
//! else's stance changes, only the player's own.

use std::collections::HashMap;
use std::sync::OnceLock;

const STANCE_DATA_JSON: &str = include_str!("../../../packs/stance_classes.json");

static STANCE_DATA: OnceLock<HashMap<String, Vec<String>>> = OnceLock::new();

/// Classes that can assume `stance`, or an empty slice if the name isn't
/// recognized -- same "unknown, not zero-eligible-classes" stance
/// `classdata::classes_for` takes for a spell it doesn't have data for.
/// Case-insensitive: the log states a stance lowercase ("an evasive
/// stance"), this file's own keys are Title Case for readability.
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
        // A real, easy mix-up: Channeler includes Druid, Evasive doesn't.
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
