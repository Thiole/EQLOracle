//! Read-side queries for the Endgame module's two Sky tabs, both built
//! from the wiki's own "Plane of Sky Class Quests" table
//! (`https://eqlwiki.com/Plane_of_Sky#Plane_of_Sky_Class_Quests`),
//! scraped by `~/eql/scrape_eqlwiki_sky_quests.py` into
//! `packs/sky_quests.json`, cross-referenced against this character's
//! own loot history, latest `/outputfile inventory` dump, and
//! `Achievements.txt`.
//!
//! Confirmed directly from the wiki page's own text, not assumed: 16
//! classes, one quest-giver NPC each, 4-7 turn-in quests per class
//! (Paladin genuinely has only 4, checked against the raw wikitext, not
//! a scrape gap), every quest needing exactly one Wind Rune plus 1-2
//! other quest items, in exchange for one gear reward. "Completing all
//! of these quests will unlock the respective class as a Primary Class
//! option in your loadouts."
//!
//! **The two tabs split on exactly that reward line, corrected directly
//! by the player after the first cut lumped everything under "Class
//! Unlocks"**: a quest's own raw materials (Wind Rune + drop items) are
//! **Sky Quests** content (`list_quests`) -- real gear-fetch quests in
//! their own right, useful even to someone who isn't chasing the class
//! unlock at all. The reward each quest produces (Mask of Song, Mantle
//! of the Songweaver, ...) is what **Primary Class Unlocks**
//! (`list_class_unlocks`) tracks -- a class unlocks once every one of
//! its own reward items shows achievement-confirmed complete, so the
//! unlock tab's own "required items" are those rewards themselves, not
//! the materials that built them.
//!
//! Three kinds of evidence per tracked item, not one:
//! - **Ever looted** -- same loot-log matching every other drop tracker
//!   in this app uses (`raiding::build_loot_index`), tier-stripped via
//!   `inventory::strip_tier` for the same reason (a quest item/reward is
//!   always a plain, untiered drop in practice, but nothing stops a
//!   future wiki drift from adding a "+N" variant, and the stripping is
//!   free either way).
//! - **Sold without keeping** -- a real, reported distinction: an item
//!   auto-sold the instant it dropped ("...and sold it for 2 platinum")
//!   was technically looted (so `ever_looted` is still true) but isn't
//!   sitting anywhere to turn in. Read off `Store::flags`'
//!   `flag::LOOT_AUTO_SOLD` bit, set at ingest time on the loot row
//!   itself (see `Ingest::record_loot`'s own doc) -- not guessed after
//!   the fact from a same-timestamp currency row, which a busy multi-
//!   item corpse could make ambiguous.
//! - **Currently owned** -- from this character's own latest
//!   `/outputfile inventory` dump (`inventory::find_existing_dump` +
//!   `inventory::parse`, the same source `gearplanner`'s owned-tier
//!   scoring already uses), which already sums bags+bank+shared bank
//!   together (see `inventory.rs`'s own doc) -- exactly "in inventory/
//!   storage" as asked for. `None`, not `0`, when no dump exists at all
//!   yet -- "unknown" and "confirmed zero" are different claims.
//!
//! Plus one more, achievement-only, not inferred: **completed** --
//! `achievements.rs`'s own "Obtain `<name>`." line (a quest's own reward
//! name, or -- same field, same meaning -- a Primary Class Unlock's own
//! reward-item name, since they're the same string). Real ground truth,
//! not a proxy the way the three above still have to be.

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

/// The wiki's own class name doesn't always match the exact text a real
/// `Achievements.txt` dump uses -- confirmed directly: the wiki page
/// (and `sky_quests.json`'s own `class` field, scraped from its `===
/// [[Shadow Knight]] Tests ===` header) says "Shadow Knight", but the
/// real achievement line reads "Primary Class Unlock - Shadowknight" --
/// one word, no space. Same class of mismatch `raiding.rs`'s own
/// `LOG_NAME_ALIASES` exists for (a real combat log calling a wiki-named
/// entity something slightly different), just against a different file.
const ACHIEVEMENT_CLASS_ALIASES: &[(&str, &str)] = &[("Shadow Knight", "Shadowknight")];

fn achievement_class_name(wiki_class: &str) -> &str {
    ACHIEVEMENT_CLASS_ALIASES
        .iter()
        .find(|&&(wiki, _)| wiki == wiki_class)
        .map(|&(_, ach)| ach)
        .unwrap_or(wiki_class)
}

/// Same class of mismatch as `ACHIEVEMENT_CLASS_ALIASES`, checked
/// against a real dump for every one of the 95 real quests: 3 reward
/// names genuinely differ from what the wiki's own `{{:RewardPage}}`
/// transclusion names, confirmed by grepping the raw Achievements text
/// directly for each -- "Fae Amulet" (wiki word order) vs "Amulet of the
/// Fae" (real achievement text); "Griffon Wing Spauldors" (a real wiki
/// page-name typo -- "Spauldors" isn't a word) vs "Griffon Wing
/// Spaulders" (real); "Windhowl" (the wiki's own transclusion links only
/// one of the two items this reward actually is) vs "Windhowl and Spirit
/// Render" (real, the achievement's own full name for it). One further
/// reward, Beastlord's own "Griffon-Hide Armguards", has no matching
/// achievement line under any wording tried -- left unaliased, reads
/// `None` honestly rather than a guessed mapping.
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
    /// Which island/boss the wiki names as this item's own source
    /// ("3-Gorga") -- free text, never parsed further (see this module's
    /// own doc on why: not a consistent island+boss shape across every
    /// entry).
    pub source: Option<String>,
    pub ever_looted: bool,
    pub looted_count: u64,
    /// `None` if no `/outputfile inventory` dump exists yet at all.
    pub currently_owned: Option<u32>,
    /// Looted at some point, but at least one copy was auto-sold on the
    /// spot rather than kept -- see this module's own doc.
    pub sold_without_keeping: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct TurnInDto {
    pub quest: String,
    pub trigger: String,
    pub rune: Option<TurnInItemDto>,
    pub items: Vec<TurnInItemDto>,
    /// The reward's own wiki page name -- not cross-referenced against
    /// `itemdata.rs`'s catalog here; the frontend can look it up by name
    /// the same way Game Data's own item search already does, if it
    /// wants stats/icon.
    pub reward: Option<String>,
    /// Real, achievement-confirmed completion -- `achievements.rs`'s own
    /// "Obtain `<reward>`." line for this exact quest, not inferred from
    /// loot/inventory the way the items above still have to be. `None`
    /// when no Achievements dump exists yet, or this reward's own line
    /// isn't in it (a wiki/achievement-file name mismatch) -- distinct
    /// from `Some(false)`, a real line that's genuinely still open.
    pub completed: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SkyClassDto {
    pub class: String,
    pub quest_giver: Option<String>,
    pub quests: Vec<TurnInDto>,
    /// Real, achievement-confirmed: this class's own "Primary Class
    /// Unlock - `<class>`" line. Same `None`-vs-`Some(false)` distinction
    /// as `TurnInDto::completed`.
    pub unlocked: Option<bool>,
}

/// One final reward item -- see this module's own doc for why this,
/// not the raw materials, is what "Primary Class Unlocks" tracks.
#[derive(Debug, Clone, Serialize)]
pub struct SkyRewardDto {
    pub name: String,
    /// Which quest earns this reward -- context for the player, this
    /// DTO's own subject is still the reward item itself.
    pub quest: String,
    pub ever_looted: bool,
    pub looted_count: u64,
    pub currently_owned: Option<u32>,
    pub sold_without_keeping: bool,
    /// Real, achievement-confirmed: this reward's own "Obtain `<name>`."
    /// line.
    pub completed: Option<bool>,
    /// The raw materials `quest` itself needs (Wind Rune first, then
    /// drop items, in the quest's own order) -- asked directly: a reward
    /// that isn't looted/owned yet needs to say *where it actually comes
    /// from* (which quest, which item, which mob/island each material
    /// drops from), not just a bare quest name. No looted/owned tracking
    /// of its own here -- that's the Sky Quests tab's own job
    /// (`TurnInDto`/`TurnInItemDto`); this is just enough to point the
    /// player at what to go do next.
    pub materials: Vec<QuestMaterialDto>,
}

/// One raw material a quest needs -- name plus where the wiki says it
/// comes from, nothing else (see `SkyRewardDto::materials`' own doc for
/// why this doesn't carry loot/ownership tracking).
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
    /// The final reward items themselves (Bard: Mask of Song, Mantle of
    /// the Songweaver, Ervaj's Flute of Flight, Amulet of the Fae,
    /// Denon's Horn of Disaster, Spear of Harmony) -- never the Wind
    /// Runes/drop items each is built from. A class unlocks once every
    /// one of these shows `completed: Some(true)`.
    pub rewards: Vec<SkyRewardDto>,
}

/// One pass over the whole store for `EventKind::Loot` rows, keyed by
/// (lowercase, tier-stripped) item name -> (total quantity looted, was
/// any copy auto-sold). Not scoped to any one target/corpse -- unlike
/// Raiding's own per-boss drop tables, a Sky quest item can plausibly
/// come from more than one source, so this indexes by item identity
/// alone. Same O(store length) cost class `by_target_and_ability`
/// already pays elsewhere in this app for the same reason (one pass
/// beats one query per item).
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

fn resolve_item(name: &str, source: Option<&str>, looted: &HashMap<String, (u64, bool)>, owned_ci: Option<&HashMap<String, u32>>) -> TurnInItemDto {
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

/// Everything both tabs need built once and shared -- one loot-index
/// scan, one inventory-dump read, one achievements-dump read, not one of
/// each per tab.
struct Context {
    looted: HashMap<String, (u64, bool)>,
    owned_ci: Option<HashMap<String, u32>>,
    achievements: Option<crate::achievements::Achievements>,
}

/// `base_dir` is the game's own install folder (same as everywhere else
/// in this app that reads `/outputfile inventory` -- `AppConfig::
/// base_dir`); `None` there, or no dump found, both correctly leave
/// `currently_owned`/`unlocked`/`completed` as `None` (unknown) rather
/// than a guessed `false`.
fn build_context(ing: &Ingest, base_dir: Option<&Path>) -> Context {
    let looted = build_item_loot_index(ing);
    let owned_ci: Option<HashMap<String, u32>> = base_dir
        .and_then(|dir| inventory::find_existing_dump(dir))
        .and_then(|(file, _character)| inventory::dump_path(base_dir.unwrap(), &file).ok())
        .and_then(|path| inventory::parse(&path).ok())
        .map(|parsed| parsed.owned.into_iter().map(|(k, v)| (k.to_ascii_lowercase(), v)).collect());
    let achievements = base_dir.and_then(crate::achievements::find_existing).and_then(|path| crate::achievements::parse(&path).ok());
    Context { looted, owned_ci, achievements }
}

fn unlocked_status(ctx: &Context, wiki_class: &str) -> Option<bool> {
    ctx.achievements
        .as_ref()
        .and_then(|a| a.is_complete(&format!("Primary Class Unlock - {}", achievement_class_name(wiki_class))))
}

/// The "Sky - Quests" tab's whole data source -- every individual
/// material turn-in (rune + drop items -> one gear reward), full detail.
/// See this module's own doc for why the *final* reward items live on a
/// separate DTO (`list_class_unlocks`) instead.
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
                    rune: q.rune.as_deref().map(|r| resolve_item(r, None, &ctx.looted, ctx.owned_ci.as_ref())),
                    items: q
                        .items
                        .iter()
                        .map(|it| resolve_item(&it.item, it.source.as_deref(), &ctx.looted, ctx.owned_ci.as_ref()))
                        .collect(),
                    completed: q
                        .reward
                        .as_ref()
                        .and_then(|r| ctx.achievements.as_ref().and_then(|a| a.is_complete(&format!("Obtain {}", achievement_reward_name(r))))),
                    reward: q.reward.clone(),
                })
                .collect(),
        })
        .collect()
}

/// The "Sky - Primary Class Unlocks" tab's whole data source -- see this
/// module's own doc for why this is *only* the final reward items, not
/// the raw materials each is built from.
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
                        .map(|r| QuestMaterialDto { item: r.to_string(), source: None })
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
                        completed: ctx
                            .achievements
                            .as_ref()
                            .and_then(|a| a.is_complete(&format!("Obtain {}", achievement_reward_name(reward)))),
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

    /// Regression check for the exact real mismatch found verifying this
    /// against a real Achievements dump: the wiki calls it "Shadow
    /// Knight", a real dump's own line reads "...Shadowknight" (one
    /// word). Confirmed via `sky_check` (`cargo run -p eqlp-app
    /// --example sky_check -- <base_dir>`) that without this alias the
    /// whole class silently read as "no achievements dump found" even
    /// with a real one present.
    #[test]
    fn shadow_knight_resolves_against_the_one_word_achievement_spelling() {
        assert_eq!(achievement_class_name("Shadow Knight"), "Shadowknight");
        assert_eq!(achievement_class_name("Wizard"), "Wizard", "everything else passes through unchanged");
    }

    /// Same regression, reward side -- the 3 known wiki/achievement
    /// reward-name drifts (`ACHIEVEMENT_REWARD_ALIASES`'s own doc) all
    /// resolve, and an ordinary reward with no known drift passes
    /// through unchanged.
    #[test]
    fn known_reward_name_drifts_resolve_to_their_real_achievement_text() {
        assert_eq!(achievement_reward_name("Fae Amulet"), "Amulet of the Fae");
        assert_eq!(achievement_reward_name("Griffon Wing Spauldors"), "Griffon Wing Spaulders");
        assert_eq!(achievement_reward_name("Windhowl"), "Windhowl and Spirit Render");
        assert_eq!(achievement_reward_name("Mask of Song"), "Mask of Song");
    }

    /// Every real class this scrape found parses, with the expected
    /// per-class quest counts confirmed against the raw wikitext
    /// directly (Paladin genuinely has only 4, not a scrape gap; most
    /// classes have 6-7).
    #[test]
    fn all_sixteen_real_classes_are_present_with_their_real_quest_counts() {
        let cs = classes();
        assert_eq!(cs.len(), 16);
        let paladin = cs.iter().find(|c| c.class == "Paladin").expect("Paladin");
        assert_eq!(paladin.quests.len(), 4);
        let bard = cs.iter().find(|c| c.class == "Bard").expect("Bard");
        assert_eq!(bard.quests.len(), 6);
    }

    /// Every real quest needs exactly one rune and at least one other
    /// item -- a parse that silently dropped the rune or came up with
    /// zero items would mean a turn-in nobody could actually complete.
    #[test]
    fn every_quest_has_a_rune_and_at_least_one_item() {
        for c in classes() {
            for q in &c.quests {
                assert!(q.rune.is_some(), "{} {} has no rune", c.class, q.quest);
                assert!(!q.items.is_empty(), "{} {} has no quest items", c.class, q.quest);
            }
        }
    }

    /// A fresh session (nothing looted, no inventory dump) reports every
    /// item honestly unresolved -- `currently_owned: None` (unknown, not
    /// zero), never looted, never sold.
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

    /// The exact case the player named directly: Bard's "Primary Class
    /// Unlocks" reward list must be exactly the 6 final gear pieces
    /// (Mask of Song, Mantle of the Songweaver, Ervaj's Flute of Flight,
    /// Amulet of the Fae, Denon's Horn of Disaster, Spear of Harmony),
    /// never the Wind Runes or raw drop items those quests are built
    /// from.
    #[test]
    fn bard_unlock_rewards_are_exactly_the_six_final_gear_pieces_not_raw_materials() {
        let ing = Ingest::default();
        let unlocks = list_class_unlocks(&ing, None);
        let bard = unlocks.iter().find(|c| c.class == "Bard").expect("Bard");
        let names: std::collections::HashSet<&str> = bard.rewards.iter().map(|r| r.name.as_str()).collect();
        // why: "Fae Amulet" (not "Amulet of the Fae") -- the wiki's own
        // reward-page name, confirmed by the scrape itself; the real
        // Achievements dump phrases it the other way around, one of the
        // handful of known wiki/achievement name drifts this module's
        // own doc already calls out (`completed` legitimately comes back
        // `None` for this one reward specifically).
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
        assert!(!names.contains("Wind Rune Meda"), "raw materials must not appear on the unlocks tab");
        assert!(!names.contains("Light Woolen Mask"), "raw materials must not appear on the unlocks tab");
    }

    /// A reward not yet secured has to say where it actually comes from
    /// -- asked directly: "list where it comes from, x quest, which
    /// mob". Real values, from Bard's own "Test of Tone" quest: rune
    /// first, then the drop item with its own real wiki source note.
    #[test]
    fn a_rewards_own_materials_name_the_real_quest_items_and_sources() {
        let ing = Ingest::default();
        let unlocks = list_class_unlocks(&ing, None);
        let bard = unlocks.iter().find(|c| c.class == "Bard").expect("Bard");
        let mask = bard.rewards.iter().find(|r| r.name == "Mask of Song").expect("Mask of Song");
        assert_eq!(mask.quest, "Bard Test of Tone");
        assert_eq!(mask.materials.len(), 2, "one rune + one drop item");
        assert_eq!(mask.materials[0].item, "Wind Rune Meda");
        assert_eq!(mask.materials[1].item, "Light Woolen Mask");
        assert_eq!(mask.materials[1].source.as_deref(), Some("3-Gorga"));
    }
}
