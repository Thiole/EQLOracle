//! why: real craft-attempt log -- every "You have fashioned.../lacked
//! the skills..." this file has ever recorded, grouped by item and
//! joined against the wiki recipe catalog (tradeskilldata) by output
//! name. Companion to that static catalog, not a replacement -- the
//! catalog says what's possible, this says what you've actually done.

use crate::ingest::Ingest;
use crate::tradeskilldata;
use eqlp_source::Millis;
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

#[derive(Debug, Clone, Serialize)]
pub struct TradeskillLevelDto {
    pub skill: String,
    /// why: None until a skill-up line has appeared this file -- the
    /// log never states a level except on a skill-up
    pub level: Option<u32>,
    pub at_ms: Option<Millis>,
    /// why: true for a tradeskill-adjacent skill with no recipe tab
    pub secondary: bool,
}

/// why: the log's name differs from the wiki catalog's for exactly one
/// skill -- confirmed in real logs ("Jewelry Making" vs "Jewelcrafting")
fn log_names(catalog_skill: &str) -> [&'static str; 2] {
    match catalog_skill {
        "Jewelcrafting" => ["Jewelry Making", "Jewelcrafting"],
        _ => ["", ""],
    }
}

/// why: leveled like tradeskills, shown in the overview, no recipe pages
const SECONDARY_SKILLS: &[&str] = &["Fishing", "Forage", "Alcohol Tolerance"];

pub fn tradeskill_levels(ing: &Ingest) -> Vec<TradeskillLevelDto> {
    let entry = |skill: &str, secondary: bool| {
        let hit = ing.skill_levels.get(skill).copied().or_else(|| {
            log_names(skill)
                .iter()
                .find_map(|n| ing.skill_levels.get(*n).copied())
        });
        TradeskillLevelDto {
            skill: skill.to_string(),
            level: hit.map(|(l, _)| l),
            at_ms: hit.map(|(_, t)| t),
            secondary,
        }
    };
    tradeskilldata::skills()
        .iter()
        .map(|s| entry(&s.skill, false))
        .chain(SECONDARY_SKILLS.iter().map(|s| entry(s, true)))
        .collect()
}

#[derive(Debug, Clone, Serialize)]
pub struct RecentCraftDto {
    pub item: String,
    pub ts_ms: Millis,
    pub tradeskill: Option<String>,
    pub trivial: Option<u32>,
    /// why: items.json icon filename, None when the item isn't known there
    pub icon: Option<String>,
}

/// why: Overview's recently-crafted list -- successes only ("crafted"
/// means actually created), newest first
pub fn recent_crafts(ing: &Ingest, limit: usize) -> Vec<RecentCraftDto> {
    let mut out = Vec::new();
    for i in (0..ing.store.len()).rev() {
        if ing.store.kind[i] != EventKind::Craft || ing.store.flags[i] & flag::CRAFT_SUCCESS == 0 {
            continue;
        }
        let item = ing.store.ability_name(ing.store.ability[i]).to_string();
        let recipe = find_recipe(&item);
        let icon = crate::itemdata::items()
            .iter()
            .find(|it| it.name.eq_ignore_ascii_case(&item))
            .and_then(|it| it.icon.clone());
        out.push(RecentCraftDto {
            ts_ms: ing.store.ts[i],
            tradeskill: recipe.map(|(s, _)| s.to_string()),
            trivial: recipe.and_then(|(_, r)| r.trivial),
            icon,
            item,
        });
        if out.len() == limit {
            break;
        }
    }
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

    /// why: real log/catalog name split -- "Jewelry Making" skill-ups
    /// must land on the catalog's "Jewelcrafting" row, last level wins
    #[test]
    fn jewelry_making_skill_ups_land_on_jewelcrafting() {
        let ing = run(
            "[Mon Aug 03 18:52:38 2026] You have become better at Jewelry Making! (2)\r\n\
             [Mon Aug 03 18:54:00 2026] You have become better at Jewelry Making! (3)\r\n",
        );
        let levels = tradeskill_levels(&ing);
        let jc = levels.iter().find(|l| l.skill == "Jewelcrafting").unwrap();
        assert_eq!(jc.level, Some(3));
        assert!(jc.at_ms.is_some());
        assert!(!jc.secondary);
    }

    /// why: a skill never seen this file is honestly unknown, and the
    /// row list always carries all 9 catalog skills plus the secondaries
    #[test]
    fn unobserved_skills_are_unknown_and_secondaries_are_listed() {
        let ing = run("[Tue Aug 18 19:50:45 2026] You have become better at Fishing! (129)\r\n");
        let levels = tradeskill_levels(&ing);
        assert_eq!(levels.len(), tradeskilldata::skills().len() + 3);
        let fishing = levels.iter().find(|l| l.skill == "Fishing").unwrap();
        assert_eq!(fishing.level, Some(129));
        assert!(fishing.secondary);
        let baking = levels.iter().find(|l| l.skill == "Baking").unwrap();
        assert_eq!(baking.level, None);
        assert_eq!(baking.at_ms, None);
    }

    /// why: a combat skill-up must not leak into the tradeskill rows
    #[test]
    fn a_non_tradeskill_skill_up_stays_out() {
        let ing = run("[Tue Jul 28 15:02:15 2026] You have become better at Meditate! (11)\r\n");
        let levels = tradeskill_levels(&ing);
        assert!(levels.iter().all(|l| l.level.is_none()));
    }

    /// why: "recently crafted" is successes only, newest first, capped --
    /// a failure between successes must not appear or break ordering
    #[test]
    fn recent_crafts_are_successes_only_newest_first_and_capped() {
        let ing = run(
            "[Tue Jul 28 15:02:15 2026] You have fashioned the items together to create something new: Ant's Potion.\r\n\
             [Tue Jul 28 15:02:18 2026] You lacked the skills to fashion Adrenaline Tap.\r\n\
             [Tue Jul 28 15:02:21 2026] You have fashioned the items together to create something new: Adrenaline Tap.\r\n",
        );
        let recent = recent_crafts(&ing, 15);
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].item, "Adrenaline Tap");
        assert_eq!(recent[1].item, "Ant's Potion");
        assert!(recent[0].ts_ms > recent[1].ts_ms);
        assert_eq!(recent[0].tradeskill.as_deref(), Some("Alchemy"));

        let capped = recent_crafts(&ing, 1);
        assert_eq!(capped.len(), 1);
        assert_eq!(capped[0].item, "Adrenaline Tap");
    }

    /// why: an item unknown to items.json crafts fine, icon stays None
    #[test]
    fn a_craft_of_an_unknown_item_has_no_icon() {
        let ing = run("[Tue Jul 28 15:02:15 2026] You have fashioned the items together to create something new: Definitely Not A Real Recipe.\r\n");
        let recent = recent_crafts(&ing, 15);
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].icon, None);
        assert_eq!(recent[0].tradeskill, None);
    }
}
