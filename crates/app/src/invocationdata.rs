//! why: invocation -> class lookup, same pattern as `stancedata.rs`
//!
//! 9 real invocations confirmed against eqlwiki. Log spelling doesn't
//! always match wiki casing ("overchannel", "empowering" for Empower) --
//! `classes_for` normalizes both sides plus that one word-form alias.

use std::collections::HashMap;
use std::sync::OnceLock;

const INVOCATION_DATA_JSON: &str = include_str!("../../../packs/invocation_classes.json");

static INVOCATION_DATA: OnceLock<HashMap<String, Vec<String>>> = OnceLock::new();
/// why: normalized name -> the JSON's own canonical key
static NORMALIZED_INDEX: OnceLock<HashMap<String, String>> = OnceLock::new();

fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_lowercase()
}

/// why: `invocation` is the raw log-text spelling, not the wiki's
pub fn classes_for(invocation: &str) -> &'static [String] {
    let map = INVOCATION_DATA.get_or_init(|| {
        serde_json::from_str(INVOCATION_DATA_JSON)
            .unwrap_or_else(|e| panic!("packs/invocation_classes.json failed to parse: {e}"))
    });
    let index = NORMALIZED_INDEX.get_or_init(|| {
        let mut idx: HashMap<String, String> =
            map.keys().map(|k| (normalize(k), k.clone())).collect();
        // why: client log prints "empowering", wiki page title is "Empower"
        idx.insert(normalize("empowering"), "Empower".to_string());
        idx
    });
    index
        .get(&normalize(invocation))
        .and_then(|canonical| map.get(canonical))
        .map(|v| v.as_slice())
        .unwrap_or(&[])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inviolable_is_bard_or_wizard() {
        let classes = classes_for("inviolable");
        assert_eq!(classes.len(), 2);
        assert!(classes.contains(&"Bard".to_string()));
        assert!(classes.contains(&"Wizard".to_string()));
    }

    #[test]
    fn real_log_spellings_resolve_despite_not_matching_the_wiki_s_own_casing() {
        assert_eq!(classes_for("overchannel"), classes_for("Over Channel"));
        assert_eq!(classes_for("spellblade"), classes_for("Spellblade"));
        assert_eq!(classes_for("empowering"), classes_for("Empower"));
        assert!(!classes_for("overchannel").is_empty());
    }

    #[test]
    fn an_unrecognized_invocation_is_unknown_not_ineligible() {
        assert!(classes_for("Not A Real Invocation").is_empty());
    }
}
