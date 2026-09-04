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
    /// why: true only when the acquisition chain proved it -- a
    /// reachable dropper whose own page lists it, or a gathered material
    /// in a reachable zone. Out of era is the default, not the exception.
    pub in_era: bool,
    /// why: the droppers that are themselves past the live era, so the
    /// row can say WHICH mob is the reason
    pub out_of_era_mobs: Vec<String>,
    /// why: WHY it isn't farmable, when no dropper can be named -- an
    /// unverifiable material and a provably gated one both read "out of
    /// era", and only this separates a real gate from a data gap. None
    /// when the acquisition chain checked out.
    pub unverified: Option<String>,
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

/// why: out of era is the DEFAULT, and in-era has to be earned through
/// the acquisition chain (Spencer: "in era means it has a means of being
/// acquired, otherwise it is out of era ... verify through the
/// acquisition chain that it can drop"). ALL of a dropper's conditions
/// have to hold, not any one: the mob has to exist, be reachable in the
/// live era, AND its own drop pool has to list the item. One such
/// dropper is enough -- you only need one place to farm it.
///
/// Every input is scraped data, so the answer moves on its own as pages
/// fill in; nothing here names a specific item or mob.
fn verify(item: &str, mobs: &[String], zone: Option<&str>, source: Option<&str>) -> Verdict {
    let live = crate::gearplanner::era_ix(crate::gearplanner::CURRENT_ERA);
    let mut earliest: Option<(usize, String)> = None;
    let mut beyond: Vec<String> = Vec::new();
    let mut unknown = 0usize;
    let mut no_pool = 0usize;
    for m in mobs {
        let Some(era) = crate::npcdata::era_of(m) else {
            unknown += 1;
            continue;
        };
        let Some(ix) = crate::gearplanner::era_ix(era) else {
            unknown += 1;
            continue;
        };
        if earliest.as_ref().is_none_or(|(b, _)| ix < *b) {
            earliest = Some((ix, era.to_string()));
        }
        if live.is_some_and(|l| ix > l) {
            beyond.push(m.clone());
            continue;
        }
        // why: reachable is only half of it -- the mob's own page has to
        // say it drops this, or nothing has verified the acquisition
        if crate::npcdata::known_loot_for(m)
            .iter()
            .any(|d| d.eq_ignore_ascii_case(item))
        {
            return Verdict {
                era: earliest.map(|(_, e)| e),
                in_era: true,
                beyond,
                unverified: None,
            };
        }
        no_pool += 1;
    }
    // why: forage and pickpocket are real acquisition means with no mob
    // to check -- the zone being reachable is the whole verification
    if source.is_some()
        && zone
            .and_then(crate::zonedata::era_of_zone)
            .and_then(crate::gearplanner::era_ix)
            .is_some_and(|ix| live.is_none_or(|l| ix <= l))
    {
        return Verdict {
            era: earliest.map(|(_, e)| e),
            in_era: true,
            beyond,
            unverified: None,
        };
    }
    let unverified = if !beyond.is_empty() {
        None
    } else if mobs.is_empty() {
        Some(match source {
            Some(s) => format!(
                "{s} source, and no era on record for {}",
                zone.unwrap_or("its zone")
            ),
            None => "no dropper, forage or pickpocket source on record".to_string(),
        })
    } else if no_pool > 0 {
        Some("its dropper's page doesn't list it as a drop".to_string())
    } else if unknown > 0 {
        Some("no era on record for any of its droppers".to_string())
    } else {
        None
    };
    Verdict {
        era: earliest.map(|(_, e)| e),
        in_era: false,
        beyond,
        unverified,
    }
}

struct Verdict {
    era: Option<String>,
    in_era: bool,
    beyond: Vec<String>,
    unverified: Option<String>,
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
                    let v = verify(&it.item, &it.mobs, it.zone.as_deref(), it.source.as_deref());
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
                        era: v.era,
                        in_era: v.in_era,
                        out_of_era_mobs: v.beyond,
                        unverified: v.unverified,
                    }
                })
                .collect(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// why: out of era is the default and in-era is earned -- a
    /// reachable dropper only counts when its OWN page lists the item,
    /// and a dropper with no page at all proves nothing
    #[test]
    fn in_era_is_earned_through_the_acquisition_chain() {
        // an Epic-Quests-Era dropper is not farmable on a Sky-Era server
        let gated = vec!["Maligar's Enraged Doppleganger".to_string()];
        let v = verify("Blade of Strategy", &gated, None, None);
        assert!(!v.in_era);
        assert_eq!(v.era.as_deref(), Some("Epic Quests Era"));
        assert_eq!(v.beyond, gated, "the dropper itself is past the live era");

        // a name with no page can never prove acquisition
        let v = verify("Anything", &["Not A Real Mob".to_string()], None, None);
        assert!(!v.in_era);
        assert!(v.beyond.is_empty());
        assert!(v.unverified.is_some(), "a data gap has to say so");

        // a reachable dropper whose own page does not list the item is
        // not verification either
        let v = verify("Not Its Drop", &["Lord Nagafen".to_string()], None, None);
        assert!(!v.in_era);
        assert!(v.unverified.is_some());

        // reachable AND its page lists it -- the only way in
        let drop = crate::npcdata::known_loot_for("Lord Nagafen")
            .first()
            .expect("Lord Nagafen has a drop pool")
            .clone();
        let v = verify(&drop, &["Lord Nagafen".to_string()], None, None);
        assert!(v.in_era, "reachable dropper that really drops it");
        assert!(v.unverified.is_none());
    }

    /// why: forage and pickpocket have no mob to check, so the zone
    /// being reachable is the whole verification
    #[test]
    fn a_gathered_material_is_verified_by_its_zone() {
        let v = verify(
            "Sweetened Mudroot",
            &[],
            Some("Greater Faydark"),
            Some("forage"),
        );
        assert!(v.in_era, "Greater Faydark is Classic Era");
        let v = verify("Whatever", &[], Some("Firiona Vie"), Some("forage"));
        assert!(!v.in_era, "Firiona Vie is Kunark Era");
        let v = verify("Whatever", &[], Some("Greater Faydark"), None);
        assert!(!v.in_era, "no stated acquisition means is not a means");
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
