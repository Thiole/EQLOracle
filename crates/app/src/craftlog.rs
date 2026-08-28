//! why: real craft-attempt log -- every "You have fashioned.../lacked
//! the skills..." this file has ever recorded, grouped by item and
//! joined against the wiki recipe catalog (tradeskilldata) by output
//! name. Companion to that static catalog, not a replacement -- the
//! catalog says what's possible, this says what you've actually done.

use crate::ingest::Ingest;
use crate::tradeskilldata;
use eqlp_store::{flag, EventKind};
use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize)]
pub struct CraftLogEntryDto {
    pub item: String,
    /// why: None when the item isn't a known recipe output anywhere in
    /// the catalog -- still a real attempt, just uncatalogued
    pub tradeskill: Option<String>,
    pub trivial: Option<u32>,
    pub attempts: u32,
    pub successes: u32,
    pub failures: u32,
    /// why: true if any attempt at this item ever hit the skill cap --
    /// a real "move on to something else" signal
    pub skill_capped: bool,
}

/// why: linear scan over the catalog per distinct crafted item, not a
/// cached index -- a real session only ever touches a handful of
/// distinct recipes, cheap enough not to bother
fn find_recipe(item_lc: &str) -> Option<(&'static str, &'static tradeskilldata::Recipe)> {
    tradeskilldata::skills().iter().find_map(|s| {
        s.recipes
            .iter()
            .find(|r| r.item.eq_ignore_ascii_case(item_lc))
            .map(|r| (s.skill.as_str(), r))
    })
}

pub fn craft_log(ing: &Ingest) -> Vec<CraftLogEntryDto> {
    let mut by_item: HashMap<String, (u32, u32, u32, bool)> = HashMap::new();
    for i in 0..ing.store.len() {
        if ing.store.kind[i] != EventKind::Craft {
            continue;
        }
        let name = ing.store.ability_name(ing.store.ability[i]).to_string();
        let success = ing.store.flags[i] & flag::CRAFT_SUCCESS != 0;
        let capped = ing.store.flags[i] & flag::CRAFT_SKILL_CAPPED != 0;
        let e = by_item.entry(name).or_insert((0, 0, 0, false));
        e.0 += 1;
        if success {
            e.1 += 1;
        } else {
            e.2 += 1;
        }
        e.3 |= capped;
    }

    let mut out: Vec<CraftLogEntryDto> = by_item
        .into_iter()
        .map(|(item, (attempts, successes, failures, skill_capped))| {
            let recipe = find_recipe(&item);
            CraftLogEntryDto {
                tradeskill: recipe.map(|(skill, _)| skill.to_string()),
                trivial: recipe.and_then(|(_, r)| r.trivial),
                item,
                attempts,
                successes,
                failures,
                skill_capped,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        b.attempts
            .cmp(&a.attempts)
            .then_with(|| a.item.cmp(&b.item))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::{backfill_lines, framed_lines};
    use crate::parser::build_engine;

    fn run(log: &str) -> Ingest {
        let engine = build_engine().expect("pack builds");
        let bytes = log.as_bytes();
        let lines = framed_lines(bytes);
        let mut ing = Ingest::default();
        backfill_lines(&mut ing, &engine, &lines, lines.len());
        ing
    }

    /// why: real bug shape -- attempts must equal successes + failures,
    /// and a real known-catalog item must resolve its tradeskill/trivial
    #[test]
    fn real_attempts_group_by_item_and_join_the_catalog() {
        let ing = run(
            "[Tue Jul 28 15:02:15 2026] You have fashioned the items together to create something new: Adrenaline Tap.\r\n\
             [Tue Jul 28 15:02:18 2026] You lacked the skills to fashion Adrenaline Tap.\r\n",
        );
        let log = craft_log(&ing);
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].item, "Adrenaline Tap");
        assert_eq!(log[0].attempts, 2);
        assert_eq!(log[0].successes, 1);
        assert_eq!(log[0].failures, 1);
        assert_eq!(log[0].tradeskill.as_deref(), Some("Alchemy"));
        assert_eq!(log[0].trivial, Some(170));
    }

    /// why: an item that's a real attempt but not in the catalog stays
    /// honestly uncatalogued, not silently dropped
    #[test]
    fn an_uncatalogued_item_still_shows_up_with_no_tradeskill() {
        let ing = run("[Tue Jul 28 15:02:15 2026] You have fashioned the items together to create something new: Definitely Not A Real Recipe.\r\n");
        let log = craft_log(&ing);
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].tradeskill, None);
        assert_eq!(log[0].trivial, None);
    }
}
