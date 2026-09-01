//! why: Endgame's Epic Quests tab, from `packs/epic_quests.json` -- an
//! ITEM-FIRST farm list, not a walkthrough. The Epic Quests Era isn't
//! open yet; the tab's job is pre-farming the loot-drop materials, so
//! every item carries the same ownership status the Sky tabs use
//! (loot history + latest inventory dump via `skyquests::Context`) and
//! the same Drop Watch bell entry points on the frontend.
//!
//! No completion tracking: there is no achievement line for these until
//! the era ships. "Done" here means "owned enough copies", nothing else.

use crate::ingest::Ingest;
use crate::skyquests::{build_context, resolve_item, TurnInItemDto};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::OnceLock;

const EPIC_QUESTS_JSON: &str = include_str!("../../../packs/epic_quests.json");

#[derive(Deserialize)]
struct RawItem {
    item: String,
    mobs: Vec<String>,
    zone: Option<String>,
    qty: u32,
    optional: bool,
    /// why: "forage"/"pickpocket" -- not a kill, shown as a hint
    source: Option<String>,
}

#[derive(Deserialize)]
struct RawClass {
    class: String,
    page: String,
    start_zone: Option<String>,
    quest_giver: Option<String>,
    recommended_level: Option<String>,
    final_reward: Option<String>,
    items: Vec<RawItem>,
}

#[derive(Deserialize)]
struct RawDoc {
    classes: Vec<RawClass>,
}

static CLASSES: OnceLock<Vec<RawClass>> = OnceLock::new();

fn classes() -> &'static [RawClass] {
    CLASSES
        .get_or_init(|| {
            let doc: RawDoc = serde_json::from_str(EPIC_QUESTS_JSON)
                .unwrap_or_else(|e| panic!("packs/epic_quests.json failed to parse: {e}"));
            doc.classes
        })
        .as_slice()
}

#[derive(Debug, Clone, Serialize)]
pub struct EpicItemDto {
    /// ownership status, same resolution as the Sky tabs
    #[serde(flatten)]
    pub status: TurnInItemDto,
    pub mobs: Vec<String>,
    pub zone: Option<String>,
    pub qty: u32,
    pub optional: bool,
    pub gather: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EpicClassDto {
    pub class: String,
    /// wiki page name -- frontend builds the link from it
    pub page: String,
    pub start_zone: Option<String>,
    pub quest_giver: Option<String>,
    pub recommended_level: Option<String>,
    pub final_reward: Option<String>,
    pub items: Vec<EpicItemDto>,
}

/// why: the Epic Quests tab's source -- every farmable material with live
/// ownership status; a class with no items (Berserker: trial spawns only,
/// nothing pre-farmable) still lists so the tab is honest about why
pub fn list_epics(ing: &Ingest, base_dir: Option<&Path>) -> Vec<EpicClassDto> {
    let ctx = build_context(ing, base_dir);
    classes()
        .iter()
        .map(|c| EpicClassDto {
            class: c.class.clone(),
            page: c.page.clone(),
            start_zone: c.start_zone.clone(),
            quest_giver: c.quest_giver.clone(),
            recommended_level: c.recommended_level.clone(),
            final_reward: c.final_reward.clone(),
            items: c
                .items
                .iter()
                .map(|it| EpicItemDto {
                    status: resolve_item(ing, &it.item, None, &ctx.looted, ctx.owned_ci.as_ref()),
                    mobs: it.mobs.clone(),
                    zone: it.zone.clone(),
                    qty: it.qty,
                    optional: it.optional,
                    gather: it.source.clone(),
                })
                .collect(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// why: all 15 classes parse; Berserker genuinely has zero farmable
    /// drops (trial spawns) but still carries its final reward
    #[test]
    fn all_fifteen_classes_parse_with_final_rewards() {
        let cs = classes();
        assert_eq!(cs.len(), 15);
        let zerker = cs
            .iter()
            .find(|c| c.class == "Berserker")
            .expect("Berserker");
        assert!(zerker.items.is_empty());
        assert_eq!(zerker.final_reward.as_deref(), Some("Kerasian Axe of Ire"));
        let rogue = cs.iter().find(|c| c.class == "Rogue").expect("Rogue");
        assert!(rogue.items.len() >= 10);
        assert_eq!(rogue.final_reward.as_deref(), Some("Ragebringer"));
    }

    /// why: fresh session reports every item honestly unknown, not zero
    #[test]
    fn a_fresh_session_reports_items_honestly_unknown() {
        let ing = Ingest::default();
        let epics = list_epics(&ing, None);
        assert_eq!(epics.len(), 15);
        let bard = epics.iter().find(|c| c.class == "Bard").expect("Bard");
        for it in &bard.items {
            assert!(!it.status.ever_looted);
            assert_eq!(it.status.currently_owned, None);
        }
    }

    /// why: the farm context survives the pipeline -- a known item keeps
    /// its kill source and zone
    #[test]
    fn a_known_item_carries_its_kill_context() {
        let ing = Ingest::default();
        let epics = list_epics(&ing, None);
        let bard = epics.iter().find(|c| c.class == "Bard").expect("Bard");
        let gut = bard
            .items
            .iter()
            .find(|i| i.status.item == "Onyx Drake Gut")
            .expect("Onyx Drake Gut");
        assert_eq!(gut.mobs, vec!["Blackwing"]);
        assert_eq!(gut.zone.as_deref(), Some("Rathe Mountains"));
        let ranger = epics.iter().find(|c| c.class == "Ranger").expect("Ranger");
        let rose = ranger
            .items
            .iter()
            .find(|i| i.status.item == "Rose of Firiona")
            .expect("Rose of Firiona");
        assert_eq!(rose.gather.as_deref(), Some("forage"));
    }
}
