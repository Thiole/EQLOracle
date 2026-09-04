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
    /// why: the earliest era any of its listed droppers belongs to, read
    /// off the MOB's own page (npcdata) -- an epic material's item page
    /// carries no era at all, so the item side cannot answer this
    pub era: Option<String>,
    /// why: false when every dropper is past the era the server is in --
    /// the material is real but unfarmable until that era ships
    pub in_era: bool,
    /// why: the droppers that are themselves past the live era, so the
    /// row can say WHICH mob is the reason
    pub out_of_era_mobs: Vec<String>,
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

/// why: an epic material's era comes from the mobs that drop it, never
/// from its own item page -- every one of the 124 materials reads as
/// era-unknown on the item side, while their droppers carry a real era
/// (Spencer: "verify on the mob who drops it's page, that the drop
/// itself is out of era, and not just on the item page itself").
/// Earliest era wins: one reachable dropper makes the material farmable.
fn drop_era(mobs: &[String]) -> (Option<String>, Vec<String>) {
    let live = crate::gearplanner::era_ix(crate::gearplanner::CURRENT_ERA);
    let mut best: Option<(usize, String)> = None;
    let mut beyond: Vec<String> = Vec::new();
    for m in mobs {
        let Some(era) = crate::npcdata::era_of(m) else {
            continue;
        };
        let Some(ix) = crate::gearplanner::era_ix(era) else {
            continue;
        };
        if live.is_some_and(|l| ix > l) {
            beyond.push(m.clone());
        }
        if best.as_ref().is_none_or(|(b, _)| ix < *b) {
            best = Some((ix, era.to_string()));
        }
    }
    // why: a dropper with no era on its page proves nothing either way,
    // so it never lands in the out-of-era list
    (best.map(|(_, e)| e), beyond)
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
                .map(|it| {
                    let (era, out_of_era_mobs) = drop_era(&it.mobs);
                    let live = crate::gearplanner::era_ix(crate::gearplanner::CURRENT_ERA);
                    let in_era = match era.as_deref().and_then(crate::gearplanner::era_ix) {
                        Some(ix) => live.is_none_or(|l| ix <= l),
                        None => true,
                    };
                    EpicItemDto {
                        status: resolve_item(
                            ing,
                            &it.item,
                            None,
                            &ctx.looted,
                            ctx.owned_ci.as_ref(),
                        ),
                        mobs: it.mobs.clone(),
                        zone: it.zone.clone(),
                        qty: it.qty,
                        optional: it.optional,
                        gather: it.source.clone(),
                        era,
                        in_era,
                        out_of_era_mobs,
                    }
                })
                .collect(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// why: the whole point of reading the MOB's page -- an epic material
    /// with an Epic-Quests-Era dropper is not farmable on a Sky-Era
    /// server, and its own item page says nothing about that
    #[test]
    fn a_droppers_own_era_decides_whether_the_material_is_farmable() {
        let mobs = vec!["Maligar's Enraged Doppleganger".to_string()];
        let (era, beyond) = drop_era(&mobs);
        assert_eq!(era.as_deref(), Some("Epic Quests Era"));
        assert_eq!(beyond, mobs, "the dropper itself is past the live era");
        // one reachable dropper is enough -- earliest era wins
        let mixed = vec![
            "Maligar's Enraged Doppleganger".to_string(),
            "Lord Nagafen".to_string(),
        ];
        let (era, beyond) = drop_era(&mixed);
        assert_eq!(era.as_deref(), Some("Classic Era"));
        assert_eq!(beyond, vec!["Maligar's Enraged Doppleganger".to_string()]);
        // a name with no page proves nothing either way
        let (era, beyond) = drop_era(&["Not A Real Mob".to_string()]);
        assert_eq!(era, None);
        assert!(beyond.is_empty());
    }

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
