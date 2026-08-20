//! Wires the scraped skill -> class lookup (`packs/skill_classes.json`)
//! into the live app, same pattern `classdata.rs`/`stancedata.rs` use.
//!
//! Deliberately small: a skill only counts as class evidence when its
//! access is purely class-gated, with no other route to it. Confirmed
//! against eqlwiki.com (fetched 2026-08-19) one skill at a time, not
//! guessed:
//! - **Tracking** -- Bard/Druid/Ranger only, no other route. Real
//!   evidence: "You have become better at Tracking! (N)" (`skill.up`)
//!   is a genuine, already-parsed log line this pack just never routed
//!   anywhere.
//! - **Forage** -- checked and deliberately left out. Bard/Druid/Ranger/
//!   Shaman get it from class, but Iksar and Wood Elf characters get it
//!   from *race* regardless of class -- and this app has no race
//!   detection at all. Treating a Forage skill-up as class-only evidence
//!   would manufacture a false positive for any Iksar/Wood Elf character
//!   playing an unrelated class. Left out until race is ever tracked,
//!   not an oversight.

use std::collections::HashMap;
use std::sync::OnceLock;

const SKILL_DATA_JSON: &str = include_str!("../../../packs/skill_classes.json");

static SKILL_DATA: OnceLock<HashMap<String, Vec<String>>> = OnceLock::new();

/// Classes that can train `skill`, or an empty slice if the name isn't
/// recognized -- same "unknown, not zero-eligible-classes" stance
/// `classdata::classes_for` takes for a spell it doesn't have data for.
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
