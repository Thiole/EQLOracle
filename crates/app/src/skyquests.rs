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
//! not 0 if no dump exists). **completed** is the achievement file's
//! "Obtain `<name>`." line, ORed with a live turn-in this session
//! (`Ingest::turn_ins`, see `confirmed_by_turnin`) -- the achievement
//! dump is only ever as fresh as the last `/outputfile`, so the live
//! signal shows a real turn-in immediately instead of waiting on that.
//!
//! Primary Class Unlocks' `currently_owned` additionally applies
//! `infer_reward_owned`: a reward is granted with no loot line and no
//! trade-offer-back line at all, so a stale dump reading 0/unknown gets
//! raised to "at least 1" once the quest is confirmed complete and the
//! reward's never been destroyed or vendor-sold since
//! (`Ingest::disposed_items`) -- never overrides a dump that already
//! shows it owned, never claims a reward that's provably gone.

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
    /// why: raw materials (rune first, then drops) with full ownership
    /// status -- player's ask: notate which components are owned, same
    /// as the Quests tab's own item chips
    pub materials: Vec<TurnInItemDto>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SkyClassUnlockDto {
    pub class: String,
    pub quest_giver: Option<String>,
    pub unlocked: Option<bool>,
    /// why: final reward items only, never raw materials; unlocks once all complete
    pub rewards: Vec<SkyRewardDto>,
}

/// why: one pass, keyed by item name -> (qty, any auto-sold, qty AFTER
/// the newest inventory dump); not scoped to a mob, since a quest item
/// can come from more than one source. The after-dump count is what
/// lets a live pickup read as owned NOW ("picked up a Blood Sky Ruby
/// and it didn't update", the report) instead of waiting for the next
/// /outputfile -- kept per-key here so resolve_item can apply it.
fn build_item_loot_index(ing: &Ingest) -> HashMap<String, (u64, bool, u64)> {
    let dump_ts = ing.last_inventory_dump_ts;
    let mut out: HashMap<String, (u64, bool, u64)> = HashMap::new();
    for i in 0..ing.store.len() {
        if ing.store.kind[i] != EventKind::Loot {
            continue;
        }
        let raw_name = ing.store.ability_name(ing.store.ability[i]);
        let (base, _tier) = inventory::strip_tier(raw_name);
        let key = base.to_ascii_lowercase();
        let sold = ing.store.flags[i] & flag::LOOT_AUTO_SOLD != 0;
        let entry = out.entry(key).or_insert((0, false, 0));
        entry.0 += ing.store.amount[i];
        entry.1 |= sold;
        // why: auto-sold loot never reached the bags -- no ownership boost
        if !sold && dump_ts.is_some_and(|d| ing.store.ts[i] > d) {
            entry.2 += ing.store.amount[i];
        }
    }
    out
}

fn resolve_item(
    ing: &Ingest,
    name: &str,
    source: Option<&str>,
    looted: &HashMap<String, (u64, bool, u64)>,
    owned_ci: Option<&HashMap<String, u32>>,
) -> TurnInItemDto {
    let key = name.to_ascii_lowercase();
    let (looted_count, sold_without_keeping, after_dump) =
        looted.get(&key).copied().unwrap_or((0, false, 0));
    TurnInItemDto {
        item: name.to_string(),
        source: source.map(str::to_string),
        ever_looted: looted_count > 0,
        looted_count,
        // why: dump count + post-dump pickups -- a live loot line is
        // real ownership the stale dump can't see. Conservative on the
        // way down: an item disposed at any point this session gets no
        // boost (disposed_items carries no timestamps to be finer with,
        // same caution infer_reward_owned takes), and no dump at all
        // stays None/unknown rather than pretending loot history is a
        // full inventory.
        currently_owned: owned_ci.map(|o| {
            let base = o.get(&key).copied().unwrap_or(0);
            let boost = if ing.disposed_items.contains(&key) {
                0
            } else {
                after_dump as u32
            };
            base + boost
        }),
        sold_without_keeping,
    }
}

/// why: everything both tabs need, built once and shared, not per-tab
struct Context {
    looted: HashMap<String, (u64, bool, u64)>,
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

/// why: live confirmation via a real trade+XP pair this session
/// (`Ingest::turn_ins`) -- ORed with the achievement file so a turn-in
/// shows complete right away, not only after a fresh Achievements dump.
/// Matches on quest giver + the exact rune+item set, tier-stripped.
fn confirmed_by_turnin(
    ing: &Ingest,
    giver: Option<&str>,
    rune: Option<&str>,
    items: &[RawQuestItem],
) -> bool {
    let Some(giver) = giver else {
        return false;
    };
    let mut wanted: Vec<String> = rune
        .into_iter()
        .chain(items.iter().map(|i| i.item.as_str()))
        .map(str::to_ascii_lowercase)
        .collect();
    wanted.sort();
    ing.turn_ins.iter().any(|t| {
        t.who == giver && {
            let mut got: Vec<String> = t
                .items
                .iter()
                .map(|(name, _qty)| inventory::strip_tier(name).0.to_ascii_lowercase())
                .collect();
            got.sort();
            got == wanted
        }
    })
}

/// why: real achievement text OR a live turn-in this session -- either one alone confirms
fn completed_status(ach: Option<bool>, live: bool) -> Option<bool> {
    if live {
        Some(true)
    } else {
        ach
    }
}

/// why: a quest reward is granted with no loot line and no "offered
/// you" line at all (confirmed against the real log) -- an inventory
/// dump is the only direct signal, and it's only ever a stale snapshot.
/// If the dump already shows it owned, trust that. Otherwise: the quest
/// completing IS the acquisition event, so if it's confirmed complete
/// and never destroyed/vendor-sold since (Ingest::disposed_items),
/// assume it's sitting somewhere -- raises an unknown/0 dump reading to
/// "at least 1", never claims ownership for a reward provably gone.
fn infer_reward_owned(
    ing: &Ingest,
    name: &str,
    dump_owned: Option<u32>,
    completed: Option<bool>,
) -> Option<u32> {
    if dump_owned.is_some_and(|n| n > 0) {
        return dump_owned;
    }
    let (base, _tier) = inventory::strip_tier(name);
    let disposed = ing.disposed_items.contains(&base.to_ascii_lowercase());
    if completed == Some(true) && !disposed {
        Some(1)
    } else {
        dump_owned
    }
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
                        .map(|r| resolve_item(ing, r, None, &ctx.looted, ctx.owned_ci.as_ref())),
                    items: q
                        .items
                        .iter()
                        .map(|it| {
                            resolve_item(
                                ing,
                                &it.item,
                                it.source.as_deref(),
                                &ctx.looted,
                                ctx.owned_ci.as_ref(),
                            )
                        })
                        .collect(),
                    completed: completed_status(
                        q.reward.as_ref().and_then(|r| {
                            ctx.achievements.as_ref().and_then(|a| {
                                a.is_complete(&format!("Obtain {}", achievement_reward_name(r)))
                            })
                        }),
                        confirmed_by_turnin(
                            ing,
                            c.quest_giver.as_deref(),
                            q.rune.as_deref(),
                            &q.items,
                        ),
                    ),
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
                    let it = resolve_item(ing, reward, None, &ctx.looted, ctx.owned_ci.as_ref());
                    let materials = q
                        .rune
                        .as_deref()
                        .map(|r| resolve_item(ing, r, None, &ctx.looted, ctx.owned_ci.as_ref()))
                        .into_iter()
                        .chain(q.items.iter().map(|qi| {
                            resolve_item(
                                ing,
                                &qi.item,
                                qi.source.as_deref(),
                                &ctx.looted,
                                ctx.owned_ci.as_ref(),
                            )
                        }))
                        .collect();
                    let completed = completed_status(
                        ctx.achievements.as_ref().and_then(|a| {
                            a.is_complete(&format!("Obtain {}", achievement_reward_name(reward)))
                        }),
                        confirmed_by_turnin(
                            ing,
                            c.quest_giver.as_deref(),
                            q.rune.as_deref(),
                            &q.items,
                        ),
                    );
                    Some(SkyRewardDto {
                        name: it.item.clone(),
                        quest: q.quest.clone(),
                        ever_looted: it.ever_looted,
                        looted_count: it.looted_count,
                        currently_owned: infer_reward_owned(
                            ing,
                            &it.item,
                            it.currently_owned,
                            completed,
                        ),
                        sold_without_keeping: it.sold_without_keeping,
                        completed,
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

    /// why: real observed log turn-in (Cilin Spellsinger, Bard Test of
    /// Voice) must mark both the quest and its reward complete with no
    /// achievements dump at all -- the live signal, not a proxy for it
    #[test]
    fn a_real_turnin_this_session_marks_its_quest_and_reward_complete_with_no_achievements_dump() {
        let mut ing = Ingest::default();
        ing.turn_ins.push(crate::ingest::ConfirmedTurnIn {
            ts: 0,
            who: "Cilin Spellsinger".to_string(),
            items: vec![
                ("Wind Rune Kala".to_string(), 1),
                ("Light Woolen Mantle".to_string(), 1),
            ],
        });

        let quests = list_quests(&ing, None);
        let bard = quests.iter().find(|c| c.class == "Bard").expect("Bard");
        let voice = bard
            .quests
            .iter()
            .find(|q| q.quest == "Bard Test of Voice")
            .expect("Bard Test of Voice");
        assert_eq!(voice.completed, Some(true));
        let tone = bard
            .quests
            .iter()
            .find(|q| q.quest == "Bard Test of Tone")
            .expect("Bard Test of Tone");
        assert_eq!(tone.completed, None, "an unrelated quest stays untouched");

        let unlocks = list_class_unlocks(&ing, None);
        let bard = unlocks.iter().find(|c| c.class == "Bard").expect("Bard");
        let mantle = bard
            .rewards
            .iter()
            .find(|r| r.name == "Mantle of the Songweaver")
            .expect("Mantle of the Songweaver");
        assert_eq!(mantle.completed, Some(true));
        assert_eq!(
            mantle.currently_owned,
            Some(1),
            "no loot line grants a reward, no dump exists either -- \
             completion itself is the only acquisition signal"
        );
    }

    /// why: a reward confirmed complete but since destroyed must NOT
    /// read as owned -- completion alone can't override real disposal
    #[test]
    fn a_completed_rewards_ownership_is_not_assumed_once_its_been_destroyed() {
        let mut ing = Ingest::default();
        ing.turn_ins.push(crate::ingest::ConfirmedTurnIn {
            ts: 0,
            who: "Cilin Spellsinger".to_string(),
            items: vec![
                ("Wind Rune Kala".to_string(), 1),
                ("Light Woolen Mantle".to_string(), 1),
            ],
        });
        ing.disposed_items
            .insert("mantle of the songweaver".to_string());

        let unlocks = list_class_unlocks(&ing, None);
        let bard = unlocks.iter().find(|c| c.class == "Bard").expect("Bard");
        let mantle = bard
            .rewards
            .iter()
            .find(|r| r.name == "Mantle of the Songweaver")
            .expect("Mantle of the Songweaver");
        assert_eq!(mantle.completed, Some(true), "completion is unaffected");
        assert_eq!(
            mantle.currently_owned, None,
            "destroyed -- no dump exists to say otherwise, so still unknown, never assumed owned"
        );
    }
}
