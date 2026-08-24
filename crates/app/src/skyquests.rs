//! why: Endgame's two Sky tabs, from the wiki's "Plane of Sky Class
//! Quests" table (`packs/sky_quests.json`), cross-referenced against
//! loot history, inventory dump, and Achievements.txt.
//!
//! 16 classes, 4-7 turn-in quests each (Paladin genuinely has 4, not a
//! scrape gap), each quest needing a Wind Rune + 1-2 items for one reward.
//!
//! Two tabs split on the reward line, per player correction: **Sky
//! Quests** (`list_quests`) tracks raw materials (Wind Rune + drop
//! items); **Primary Class Unlocks** (`list_class_unlocks`) tracks the
//! final reward items themselves -- a class unlocks once every reward
//! shows achievement-confirmed complete.
//!
//! Per tracked item: ever looted (tier-stripped), sold-without-keeping
//! (from `flag::LOOT_AUTO_SOLD`, not guessed from a same-timestamp
//! currency row), currently owned (from the latest inventory dump, None
//! not 0 if no dump exists). Plus achievement-only **completed**, real
//! ground truth via "Obtain `<name>`." lines, not a proxy.

use crate::ingest::Ingest;
use crate::inventory;
use eqlp_store::{flag, EventKind};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

const SKY_QUESTS_JSON: &str = include_str!("../../../packs/sky_quests.json");

#[derive(Deserialize)]
struct RawQuestItem {
    item: String,
    source: Option<String>,
}

#[derive(Deserialize)]
struct RawQuest {
    quest: String,
    trigger: String,
    rune: Option<String>,
    items: Vec<RawQuestItem>,
    reward: Option<String>,
}

#[derive(Deserialize)]
struct RawClass {
    class: String,
    quest_giver: Option<String>,
    quests: Vec<RawQuest>,
}

#[derive(Deserialize)]
struct RawDoc {
    classes: Vec<RawClass>,
}

/// why: wiki says "Shadow Knight", real achievement line says
/// "Shadowknight" one word -- same class of mismatch as `raiding.rs`'s
/// `LOG_NAME_ALIASES`, against a different file
const ACHIEVEMENT_CLASS_ALIASES: &[(&str, &str)] = &[("Shadow Knight", "Shadowknight")];

fn achievement_class_name(wiki_class: &str) -> &str {
    ACHIEVEMENT_CLASS_ALIASES
        .iter()
        .find(|&&(wiki, _)| wiki == wiki_class)
        .map(|&(_, ach)| ach)
        .unwrap_or(wiki_class)
}

/// why: 3 confirmed reward-name drifts between wiki and real Achievements
/// text (word order, a wiki typo, an incomplete transclusion). Beastlord's
/// "Griffon-Hide Armguards" has no match under any wording -- left
/// unaliased, reads None honestly rather than a guessed mapping.
const ACHIEVEMENT_REWARD_ALIASES: &[(&str, &str)] = &[
    ("Fae Amulet", "Amulet of the Fae"),
    ("Griffon Wing Spauldors", "Griffon Wing Spaulders"),
    ("Windhowl", "Windhowl and Spirit Render"),
];

fn achievement_reward_name(wiki_reward: &str) -> &str {
    ACHIEVEMENT_REWARD_ALIASES
        .iter()
        .find(|&&(wiki, _)| wiki == wiki_reward)
        .map(|&(_, ach)| ach)
        .unwrap_or(wiki_reward)
}

static CLASSES: OnceLock<Vec<RawClass>> = OnceLock::new();

fn classes() -> &'static [RawClass] {
    CLASSES
        .get_or_init(|| {
            let doc: RawDoc = serde_json::from_str(SKY_QUESTS_JSON)
                .unwrap_or_else(|e| panic!("packs/sky_quests.json failed to parse: {e}"));
            doc.classes
        })
        .as_slice()
}

#[derive(Debug, Clone, Serialize)]
pub struct TurnInItemDto {
    pub item: String,
    /// why: free text island/boss source, never parsed further -- not a consistent shape
    pub source: Option<String>,
    pub ever_looted: bool,
    pub looted_count: u64,
    /// why: None if no inventory dump exists yet
    pub currently_owned: Option<u32>,
    /// why: looted at some point but at least one copy auto-sold on the spot
    pub sold_without_keeping: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct TurnInDto {
    pub quest: String,
    pub trigger: String,
    pub rune: Option<TurnInItemDto>,
    pub items: Vec<TurnInItemDto>,
    /// why: wiki page name, not cross-referenced here -- frontend looks it up by name
    pub reward: Option<String>,
    /// why: real achievement-confirmed "Obtain <reward>." line, not
    /// inferred; None distinct from Some(false)
    pub completed: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SkyClassDto {
    pub class: String,
    pub quest_giver: Option<String>,
    pub quests: Vec<TurnInDto>,
    /// why: real "Primary Class Unlock - <class>" line, same None/Some(false) distinction
    pub unlocked: Option<bool>,
}

/// why: final reward item, what "Primary Class Unlocks" tracks, not raw materials
#[derive(Debug, Clone, Serialize)]
pub struct SkyRewardDto {
    pub name: String,
    /// why: which quest earns this reward -- context, not the DTO's subject
    pub quest: String,
    pub ever_looted: bool,
    pub looted_count: u64,
    pub currently_owned: Option<u32>,
    pub sold_without_keeping: bool,
    /// why: real achievement-confirmed "Obtain <name>." line
    pub completed: Option<bool>,
    /// why: raw materials (rune first, then drops) so a not-yet-owned
    /// reward says where it actually comes from; no own tracking here
    pub materials: Vec<QuestMaterialDto>,
}

/// why: name + wiki source, nothing else -- no loot/ownership tracking here
#[derive(Debug, Clone, Serialize)]
pub struct QuestMaterialDto {
    pub item: String,
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SkyClassUnlockDto {
    pub class: String,
    pub quest_giver: Option<String>,
    pub unlocked: Option<bool>,
    /// why: final reward items only, never raw materials; unlocks once all complete
    pub rewards: Vec<SkyRewardDto>,
}

/// why: one pass, keyed by item name -> (qty, any auto-sold); not scoped
/// to a mob, since a quest item can come from more than one source
fn build_item_loot_index(ing: &Ingest) -> HashMap<String, (u64, bool)> {
    let mut out: HashMap<String, (u64, bool)> = HashMap::new();
    for i in 0..ing.store.len() {
        if ing.store.kind[i] != EventKind::Loot {
            continue;
        }
        let raw_name = ing.store.ability_name(ing.store.ability[i]);
        let (base, _tier) = inventory::strip_tier(raw_name);
        let key = base.to_ascii_lowercase();
        let sold = ing.store.flags[i] & flag::LOOT_AUTO_SOLD != 0;
        let entry = out.entry(key).or_insert((0, false));
        entry.0 += ing.store.amount[i];
        entry.1 |= sold;
    }
    out
}

fn resolve_item(
    name: &str,
    source: Option<&str>,
    looted: &HashMap<String, (u64, bool)>,
    owned_ci: Option<&HashMap<String, u32>>,
) -> TurnInItemDto {
    let key = name.to_ascii_lowercase();
    let (looted_count, sold_without_keeping) = looted.get(&key).copied().unwrap_or((0, false));
    TurnInItemDto {
        item: name.to_string(),
        source: source.map(str::to_string),
        ever_looted: looted_count > 0,
        looted_count,
        currently_owned: owned_ci.map(|o| o.get(&key).copied().unwrap_or(0)),
        sold_without_keeping,
    }
}

/// why: everything both tabs need, built once and shared, not per-tab
struct Context {
    looted: HashMap<String, (u64, bool)>,
    owned_ci: Option<HashMap<String, u32>>,
    achievements: Option<crate::achievements::Achievements>,
}

/// why: None base_dir or no dump found both leave fields None (unknown), not guessed false
fn build_context(ing: &Ingest, base_dir: Option<&Path>) -> Context {
    let looted = build_item_loot_index(ing);
    let owned_ci: Option<HashMap<String, u32>> = base_dir
        .and_then(inventory::find_existing_dump)
        .and_then(|(file, _character)| inventory::dump_path(base_dir.unwrap(), &file).ok())
        .and_then(|path| inventory::parse(&path).ok())
        .map(|parsed| {
            parsed
                .owned
                .into_iter()
                .map(|(k, v)| (k.to_ascii_lowercase(), v))
                .collect()
        });
    let achievements = base_dir
        .and_then(crate::achievements::find_existing)
        .and_then(|path| crate::achievements::parse(&path).ok());
    Context {
        looted,
        owned_ci,
        achievements,
    }
}

fn unlocked_status(ctx: &Context, wiki_class: &str) -> Option<bool> {
    ctx.achievements.as_ref().and_then(|a| {
        a.is_complete(&format!(
            "Primary Class Unlock - {}",
            achievement_class_name(wiki_class)
        ))
    })
}

/// why: "Sky - Quests" tab's source -- every material turn-in, full detail
pub fn list_quests(ing: &Ingest, base_dir: Option<&Path>) -> Vec<SkyClassDto> {
    let ctx = build_context(ing, base_dir);
    classes()
        .iter()
        .map(|c| SkyClassDto {
            class: c.class.clone(),
            quest_giver: c.quest_giver.clone(),
            unlocked: unlocked_status(&ctx, &c.class),
            quests: c
                .quests
                .iter()
                .map(|q| TurnInDto {
                    quest: q.quest.clone(),
                    trigger: q.trigger.clone(),
                    rune: q
                        .rune
                        .as_deref()
                        .map(|r| resolve_item(r, None, &ctx.looted, ctx.owned_ci.as_ref())),
                    items: q
                        .items
                        .iter()
                        .map(|it| {
                            resolve_item(
                                &it.item,
                                it.source.as_deref(),
                                &ctx.looted,
                                ctx.owned_ci.as_ref(),
                            )
                        })
                        .collect(),
                    completed: q.reward.as_ref().and_then(|r| {
                        ctx.achievements.as_ref().and_then(|a| {
                            a.is_complete(&format!("Obtain {}", achievement_reward_name(r)))
                        })
                    }),
                    reward: q.reward.clone(),
                })
                .collect(),
        })
        .collect()
}

/// why: "Sky - Primary Class Unlocks" tab's source -- final reward items only
pub fn list_class_unlocks(ing: &Ingest, base_dir: Option<&Path>) -> Vec<SkyClassUnlockDto> {
    let ctx = build_context(ing, base_dir);
    classes()
        .iter()
        .map(|c| {
            let rewards = c
                .quests
                .iter()
                .filter_map(|q| {
                    let reward = q.reward.as_deref()?;
                    let it = resolve_item(reward, None, &ctx.looted, ctx.owned_ci.as_ref());
                    let materials = q
                        .rune
                        .as_deref()
                        .map(|r| QuestMaterialDto {
                            item: r.to_string(),
                            source: None,
                        })
                        .into_iter()
                        .chain(q.items.iter().map(|qi| QuestMaterialDto {
                            item: qi.item.clone(),
                            source: qi.source.clone(),
                        }))
                        .collect();
                    Some(SkyRewardDto {
                        name: it.item,
                        quest: q.quest.clone(),
                        ever_looted: it.ever_looted,
                        looted_count: it.looted_count,
                        currently_owned: it.currently_owned,
                        sold_without_keeping: it.sold_without_keeping,
                        completed: ctx.achievements.as_ref().and_then(|a| {
                            a.is_complete(&format!("Obtain {}", achievement_reward_name(reward)))
                        }),
                        materials,
                    })
                })
                .collect();
            SkyClassUnlockDto {
                class: c.class.clone(),
                quest_giver: c.quest_giver.clone(),
                unlocked: unlocked_status(&ctx, &c.class),
                rewards,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// why: regression check -- wiki "Shadow Knight" vs real "Shadowknight"
    #[test]
    fn shadow_knight_resolves_against_the_one_word_achievement_spelling() {
        assert_eq!(achievement_class_name("Shadow Knight"), "Shadowknight");
        assert_eq!(
            achievement_class_name("Wizard"),
            "Wizard",
            "everything else passes through unchanged"
        );
    }

    /// why: same regression, reward side -- 3 known drifts resolve, ordinary passes through
    #[test]
    fn known_reward_name_drifts_resolve_to_their_real_achievement_text() {
        assert_eq!(achievement_reward_name("Fae Amulet"), "Amulet of the Fae");
        assert_eq!(
            achievement_reward_name("Griffon Wing Spauldors"),
            "Griffon Wing Spaulders"
        );
        assert_eq!(
            achievement_reward_name("Windhowl"),
            "Windhowl and Spirit Render"
        );
        assert_eq!(achievement_reward_name("Mask of Song"), "Mask of Song");
    }

    /// why: every real class parses with real per-class quest counts
    #[test]
    fn all_sixteen_real_classes_are_present_with_their_real_quest_counts() {
        let cs = classes();
        assert_eq!(cs.len(), 16);
        let paladin = cs.iter().find(|c| c.class == "Paladin").expect("Paladin");
        assert_eq!(paladin.quests.len(), 4);
        let bard = cs.iter().find(|c| c.class == "Bard").expect("Bard");
        assert_eq!(bard.quests.len(), 6);
    }

    /// why: every quest needs a rune and an item -- a silent drop would break the turn-in
    #[test]
    fn every_quest_has_a_rune_and_at_least_one_item() {
        for c in classes() {
            for q in &c.quests {
                assert!(q.rune.is_some(), "{} {} has no rune", c.class, q.quest);
                assert!(
                    !q.items.is_empty(),
                    "{} {} has no quest items",
                    c.class,
                    q.quest
                );
            }
        }
    }

    /// why: fresh session reports every item honestly unresolved, not zero
    #[test]
    fn a_fresh_session_with_no_inventory_dump_reports_every_item_honestly_unknown() {
        let ing = Ingest::default();
        let quests = list_quests(&ing, None);
        assert!(!quests.is_empty());
        for c in &quests {
            for q in &c.quests {
                for item in q.rune.iter().chain(q.items.iter()) {
                    assert!(!item.ever_looted);
                    assert_eq!(item.looted_count, 0);
                    assert_eq!(item.currently_owned, None);
                    assert!(!item.sold_without_keeping);
                }
            }
        }
    }

    /// why: Bard's unlock rewards must be exactly the 6 gear pieces, never raw materials
    #[test]
    fn bard_unlock_rewards_are_exactly_the_six_final_gear_pieces_not_raw_materials() {
        let ing = Ingest::default();
        let unlocks = list_class_unlocks(&ing, None);
        let bard = unlocks.iter().find(|c| c.class == "Bard").expect("Bard");
        let names: std::collections::HashSet<&str> =
            bard.rewards.iter().map(|r| r.name.as_str()).collect();
        // why: "Fae Amulet" is the wiki's own name, real Achievements text
        // phrases it the other way -- one of the known drifts
        let expected: std::collections::HashSet<&str> = [
            "Mask of Song",
            "Mantle of the Songweaver",
            "Ervaj's Flute of Flight",
            "Fae Amulet",
            "Denon's Horn of Disaster",
            "Spear of Harmony",
        ]
        .into_iter()
        .collect();
        assert_eq!(names, expected);
        assert!(
            !names.contains("Wind Rune Meda"),
            "raw materials must not appear on the unlocks tab"
        );
        assert!(
            !names.contains("Light Woolen Mask"),
            "raw materials must not appear on the unlocks tab"
        );
    }

    /// why: a not-yet-secured reward must say where it comes from, real values
    #[test]
    fn a_rewards_own_materials_name_the_real_quest_items_and_sources() {
        let ing = Ingest::default();
        let unlocks = list_class_unlocks(&ing, None);
        let bard = unlocks.iter().find(|c| c.class == "Bard").expect("Bard");
        let mask = bard
            .rewards
            .iter()
            .find(|r| r.name == "Mask of Song")
            .expect("Mask of Song");
        assert_eq!(mask.quest, "Bard Test of Tone");
        assert_eq!(mask.materials.len(), 2, "one rune + one drop item");
        assert_eq!(mask.materials[0].item, "Wind Rune Meda");
        assert_eq!(mask.materials[1].item, "Light Woolen Mask");
        assert_eq!(mask.materials[1].source.as_deref(), Some("3-Gorga"));
    }
}
