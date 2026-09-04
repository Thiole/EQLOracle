//! why: monster -> known-drops lookup, same pattern as `classdata.rs`
//!
//! Built from `items.json`'s `drops[].mobs` field -- a mob dropping
//! nothing recorded simply isn't in this file (not "not a real monster",
//! that's `Ingest::link`'s job). See `crate::monsters::list_mobs`.

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
            // why: malformed embedded data is a build bug, fail loud
            panic!("packs/monsters.json failed to parse: {e}")
        });
        file.mobs
    })
}

/// why: mirrors `eqlp_session::fold_key` (pub(crate) there, not importable);
/// store names keep first-seen casing, this normalizes both sides to match
use eqlp_session::fold_key;

/// why: whether the wiki's drop data ever named `name` as a source
pub fn is_known_monster(name: &str) -> bool {
    monster_data().contains_key(&fold_key(name))
}

/// why: empty is ambiguous (unknown mob vs known, no drops); check
/// `is_known_monster` first if that distinction matters
pub fn known_drops(name: &str) -> &'static [String] {
    monster_data()
        .get(&fold_key(name))
        .map(|v| v.as_slice())
        .unwrap_or(&[])
}
