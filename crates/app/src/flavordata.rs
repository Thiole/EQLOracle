//! Wires the scraped buff-landing-message -> class lookup
//! (`packs/spell_flavor.json`) into the live app, the same way
//! `classdata.rs` wires spell -> class data in.
//!
//! `packs/spell_flavor.json` is generated, not hand-written -- see
//! `~/eql/build_spell_flavor.py`. It exists for exactly one purpose: Quick
//! Buff (an AA, class-agnostic itself) silently applies whatever buffs the
//! activator actually knows, with no "begins casting" line for any of them
//! -- only each buff's own first-person landing flavor text
//! ("A burst of strength surges through your body.") is left behind. See
//! `Ingest`'s quickbuff-window handling for how this gets applied safely
//! (only to unmatched lines shortly after a confirmed activation, never as
//! a general "any flavor text anywhere is evidence" rule -- a landing
//! message says nothing about who cast it, so outside that narrow window
//! it's not attributable).

use std::collections::HashMap;
use std::sync::OnceLock;

const FLAVOR_DATA_JSON: &str = include_str!("../../../packs/spell_flavor.json");

static FLAVOR_DATA: OnceLock<HashMap<String, Vec<String>>> = OnceLock::new();

/// Classes a spell whose first-person landing message is exactly `text`
/// can belong to, or an empty slice if `text` isn't a recognised landing
/// message at all.
pub fn classes_for_flavor(text: &str) -> &'static [String] {
    let map = FLAVOR_DATA.get_or_init(|| {
        serde_json::from_str(FLAVOR_DATA_JSON)
            .unwrap_or_else(|e| panic!("packs/spell_flavor.json failed to parse: {e}"))
    });
    map.get(text).map(|v| v.as_slice()).unwrap_or(&[])
}

/// Every known first-person landing message, verbatim -- the dictionary's
/// own key set. For a caller that needs to *derive* something from the
/// text itself (`crate::ingest`'s third-person recognizers build their own
/// suffix tables from this), not just look one exact string up.
pub fn all_texts() -> impl Iterator<Item = &'static str> {
    let map = FLAVOR_DATA.get_or_init(|| {
        serde_json::from_str(FLAVOR_DATA_JSON)
            .unwrap_or_else(|e| panic!("packs/spell_flavor.json failed to parse: {e}"))
    });
    map.keys().map(String::as_str)
}
