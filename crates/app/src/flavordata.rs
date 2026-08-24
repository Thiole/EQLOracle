//! why: buff-landing-message -> class lookup, for Quick Buff's silent casts
//!
//! Quick Buff applies buffs with no "begins casting" line, only landing
//! flavor text. `Ingest`'s quickbuff-window attributes it, only briefly
//! after a confirmed activation -- flavor text alone names no caster.
//! Generated from `~/eql/build_spell_flavor.py`, not hand-written.

use std::collections::HashMap;
use std::sync::OnceLock;

const FLAVOR_DATA_JSON: &str = include_str!("../../../packs/spell_flavor.json");

static FLAVOR_DATA: OnceLock<HashMap<String, Vec<String>>> = OnceLock::new();

/// why: classes for a landing message, empty slice if unrecognized
pub fn classes_for_flavor(text: &str) -> &'static [String] {
    let map = FLAVOR_DATA.get_or_init(|| {
        serde_json::from_str(FLAVOR_DATA_JSON)
            .unwrap_or_else(|e| panic!("packs/spell_flavor.json failed to parse: {e}"))
    });
    map.get(text).map(|v| v.as_slice()).unwrap_or(&[])
}

/// why: full key set, for callers deriving suffix tables not exact lookups
pub fn all_texts() -> impl Iterator<Item = &'static str> {
    let map = FLAVOR_DATA.get_or_init(|| {
        serde_json::from_str(FLAVOR_DATA_JSON)
            .unwrap_or_else(|e| panic!("packs/spell_flavor.json failed to parse: {e}"))
    });
    map.keys().map(String::as_str)
}
