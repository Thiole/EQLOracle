//! Wires the scraped invocation -> class lookup
//! (`packs/invocation_classes.json`) into the live app, same pattern
//! `stancedata.rs` uses for stances.
//!
//! Confirmed against eqlwiki.com's "Stances & Invocations" page (fetched
//! 2026-08-19), not guessed: 9 real invocations, each with its own class
//! list -- "Inviolable" (Bard/Wizard) and "Spellblade" (Beastlord/
//! Paladin/Ranger/Shadow Knight) are narrow enough to be real evidence;
//! "Inversion"/"Over Channel"/"Recovery" are essentially "every caster or
//! hybrid class" and rarely narrow anything on their own, but cost
//! nothing to include -- they're just as real, the elimination step
//! either uses them or doesn't.
//!
//! Real log line: "You begin reciting the <name> invocation." Checked
//! against a real character's own log for the exact spelling each
//! invocation actually prints, which doesn't always match the wiki's own
//! page-title casing/spacing: "overchannel" and "spellblade" print as one
//! word (not "Over Channel"/"Spellblade"), and "Empower" prints as
//! "empowering". `classes_for` normalizes both sides (lowercase, spaces
//! stripped) plus that one real word-form alias, rather than trusting the
//! log to match the wiki's own prose exactly.

use std::collections::HashMap;
use std::sync::OnceLock;

const INVOCATION_DATA_JSON: &str = include_str!("../../../packs/invocation_classes.json");

static INVOCATION_DATA: OnceLock<HashMap<String, Vec<String>>> = OnceLock::new();
/// Normalized (lowercase, no spaces) name -> the JSON's own canonical key.
static NORMALIZED_INDEX: OnceLock<HashMap<String, String>> = OnceLock::new();

fn normalize(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_lowercase()
}

/// Classes that grant `invocation`, or an empty slice if the name isn't
/// recognized. `invocation` is the raw log-text name ("overchannel",
/// "empowering", ...), not necessarily the wiki's own spelling.
pub fn classes_for(invocation: &str) -> &'static [String] {
    let map = INVOCATION_DATA.get_or_init(|| {
        serde_json::from_str(INVOCATION_DATA_JSON)
            .unwrap_or_else(|e| panic!("packs/invocation_classes.json failed to parse: {e}"))
    });
    let index = NORMALIZED_INDEX.get_or_init(|| {
        let mut idx: HashMap<String, String> = map.keys().map(|k| (normalize(k), k.clone())).collect();
        // Real word-form difference confirmed in a real log: the client
        // prints "empowering", the wiki's own page title is "Empower".
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
