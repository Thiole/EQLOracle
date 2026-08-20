//! Wires the scraped monster -> known-drops lookup (`packs/monsters.json`)
//! into the live app, the same way `classdata.rs` wires spell -> class data
//! in.
//!
//! `packs/monsters.json` is generated, not hand-written -- see
//! `~/eql/build_monster_drops.py` on the scraping machine. It is a
//! snapshot built from `items.json`'s own `drops[].mobs` field, the only
//! place a mob name exists anywhere in the scraped wiki data: **a mob that
//! drops nothing the scrape recorded simply isn't in this file at all.**
//! That is not the same claim as "not a real monster" -- `Ingest::link`'s
//! `Kind`-based check is what decides whether something fought is a mob
//! worth tracking at all (see its doc comment), independent of this file.
//! This lookup exists only to answer a narrower question once that's
//! already settled: for a mob known to the wiki, what can it drop, and
//! which of those has this player actually gotten -- see
//! `crate::monsters::list_mobs` for how the two combine.

use std::collections::HashMap;
use std::sync::OnceLock;

const MONSTER_DATA_JSON: &str = include_str!("../../../packs/monsters.json");

#[derive(serde::Deserialize)]
struct MonsterDataFile {
    mobs: HashMap<String, Vec<String>>,
}

static MONSTER_DATA: OnceLock<HashMap<String, Vec<String>>> = OnceLock::new();

fn monster_data() -> &'static HashMap<String, Vec<String>> {
    MONSTER_DATA.get_or_init(|| {
        let file: MonsterDataFile = serde_json::from_str(MONSTER_DATA_JSON).unwrap_or_else(|e| {
            // A malformed embedded file is a build-time data bug, not a
            // runtime condition to recover from gracefully -- same stance
            // as classdata.rs and a bad rule pack failing `build_engine`.
            panic!("packs/monsters.json failed to parse: {e}")
        });
        file.mobs
    })
}

/// Folds a name the same way `eqlp_session::fold_key` folds combat-log
/// identities (lowercase the first character only, leave the rest alone),
/// duplicated here rather than imported -- that function is `pub(crate)` to
/// `eqlp-session`, and this is one line, not worth widening its visibility
/// for. `packs/monsters.json`'s keys are pre-folded the same way at build
/// time (see `build_monster_drops.py`), so both sides of the lookup use one
/// algorithm even though neither can literally call the other's copy.
/// Needed because `Ingest`'s own store names keep whatever casing an entity
/// was *first* seen under (`Entities::display_name`), which can land either
/// side of sentence-start capitalisation unpredictably -- exact-string
/// matching against the wiki's own inconsistently-cased scrape would miss
/// real matches on nothing but casing.
fn fold_key(name: &str) -> String {
    let mut c = name.chars();
    match c.next() {
        Some(f) => f.to_lowercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// Whether the wiki's item-drop data ever named `name` as a source. See the
/// module doc for what absence does and doesn't mean.
pub fn is_known_monster(name: &str) -> bool {
    monster_data().contains_key(&fold_key(name))
}

/// Items the wiki lists as dropping from `name`, or an empty slice if
/// `name` isn't a known monster. Empty is ambiguous between "unknown mob"
/// and "known mob, wiki records no drops" the same way it is in
/// `classdata::classes_for`; use `is_known_monster` first if that
/// distinction matters to the caller.
pub fn known_drops(name: &str) -> &'static [String] {
    monster_data()
        .get(&fold_key(name))
        .map(|v| v.as_slice())
        .unwrap_or(&[])
}
