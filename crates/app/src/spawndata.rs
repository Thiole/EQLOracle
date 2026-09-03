//! why: the in-game spawn survey, packed -- `/say %t` + `/loc` pairs from
//! a real log (tools/scrapers/build_spawns.py), one point per mob per
//! spot, with a z. The wiki's spawn spots are XY only and thin; a
//! surveyed point is the game's own answer and comes first everywhere
//! spawn points are used (map markers, "set path here", the mob list).

use serde::Deserialize;
use std::sync::OnceLock;

const SPAWN_DATA_JSON: &str = include_str!("../../../packs/spawns.json");

#[derive(Debug, Clone, Deserialize)]
pub struct Spawn {
    /// why: the RAW log zone name ("Clan Crushbone"); matched to a wiki
    /// zone with the same alias fold everything else uses
    pub zone: String,
    /// why: as `%t` printed it -- usually without the article the wiki
    /// carries ("orc centurion" vs "an orc centurion"); see `same_mob`
    pub name: String,
    /// why: /loc coordinates, the same space the wiki spots are in
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub count: u32,
    pub invis: bool,
}

pub fn spawns() -> &'static [Spawn] {
    static DATA: OnceLock<Vec<Spawn>> = OnceLock::new();
    DATA.get_or_init(|| {
        serde_json::from_str(SPAWN_DATA_JSON)
            .unwrap_or_else(|e| panic!("packs/spawns.json failed to parse: {e}"))
    })
}

/// why: "orc centurion", "an orc centurion", "An Orc Centurion" are one mob
pub fn fold_mob_name(name: &str) -> String {
    let lower = name.trim().to_ascii_lowercase();
    for article in ["an ", "a ", "the "] {
        if let Some(rest) = lower.strip_prefix(article) {
            return rest.to_string();
        }
    }
    lower
}

pub fn same_mob(a: &str, b: &str) -> bool {
    fold_mob_name(a) == fold_mob_name(b)
}

/// why: every surveyed point in a wiki zone (alias-folded), /loc space
pub fn spawns_in_wiki_zone(wiki_zone: &str) -> impl Iterator<Item = &'static Spawn> + '_ {
    spawns()
        .iter()
        .filter(move |s| crate::zone::zone_matches(&s.zone, wiki_zone))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pack_parses_and_folds_articles() {
        assert!(same_mob("orc centurion", "an orc centurion"));
        assert!(same_mob("A dwarven smith", "a dwarven smith"));
        assert!(!same_mob("orc centurion", "orc legionnaire"));
        // why: the first survey -- Crushbone, keyed by the raw log zone
        assert!(spawns_in_wiki_zone("Crushbone").any(|s| same_mob(&s.name, "Emperor Crush")));
    }
}
