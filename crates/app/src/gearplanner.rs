//! why: native gear planner -- item browsing, BiS-style recommendation,
//! pre-selects classes from the live parse.
//!
//! Scoring model is a faithful port of the standalone planner
//! (`ui/app/planner/`), a heuristic opinion not verified game data.
//! Deliberately a subset -- 2H/dual-wield hand-pairing and exaltation
//! auto-assignment aren't ported. INT/WIS diverge from the standalone's
//! flat number though -- see `derived_weights` for the mana-pool
//! mechanic that flat weight couldn't see, confirmed against a real character.
//!
//! LORE duplicates handled via a single greedy pass over SLOTS
//! (`recommend`'s `claimed_lore`), not the standalone's fuller
//! coverage-optimizing assignment -- never suggests unwearable, not guaranteed globally best.

use crate::ingest::ExaltationProcs;
use crate::itemdata::{self, Item};
use eqlp_source::Millis;
use serde::Serialize;
use std::collections::HashMap;

// ---------------------------------------------------------------- classes/races

/// why: class code -> full name, same roster as the standalone planner
pub const CLASS_NAMES: &[(&str, &str)] = &[
    ("WAR", "Warrior"),
    ("CLR", "Cleric"),
    ("PAL", "Paladin"),
    ("RNG", "Ranger"),
    ("SHD", "Shadow Knight"),
    ("DRU", "Druid"),
    ("MNK", "Monk"),
    ("BRD", "Bard"),
    ("ROG", "Rogue"),
    ("SHM", "Shaman"),
    ("NEC", "Necromancer"),
    ("WIZ", "Wizard"),
    ("MAG", "Magician"),
    ("ENC", "Enchanter"),
    ("BST", "Beastlord"),
    ("BER", "Berserker"),
];

fn code_to_name(code: &str) -> Option<&'static str> {
    CLASS_NAMES
        .iter()
        .find(|(c, _)| *c == code)
        .map(|(_, n)| *n)
}
/// why: pub(crate) so `crate::character` shares this instead of a second copy
pub(crate) fn name_to_code(name: &str) -> Option<&'static str> {
    CLASS_NAMES
        .iter()
        .find(|(_, n)| *n == name)
        .map(|(c, _)| *c)
}

/// why: module-level const so `crate::character` shares this instead of a drifting copy
pub(crate) const RACE_NAMES: &[(&str, &str)] = &[
    ("HUM", "Human"),
    ("BAR", "Barbarian"),
    ("ERU", "Erudite"),
    ("ELF", "Wood Elf"),
    ("HIE", "High Elf"),
    ("DEF", "Dark Elf"),
    ("HFL", "Halfling"),
    ("DWF", "Dwarf"),
    ("TRL", "Troll"),
    ("OGR", "Ogre"),
    ("GNM", "Gnome"),
    ("IKS", "Iksar"),
    ("VAH", "Vah Shir"),
    ("FRG", "Froglok"),
    ("HEF", "Half Elf"),
];

pub(crate) fn race_name_to_code(name: &str) -> Option<&'static str> {
    RACE_NAMES.iter().find(|(_, n)| *n == name).map(|(c, _)| *c)
}

/// why: full names -> this module's codes; unmatched names dropped, not guessed
fn names_to_codes(names: &[String]) -> Vec<String> {
    names
        .iter()
        .filter_map(|n| name_to_code(n))
        .map(String::from)
        .collect()
}

/// why: full class names, matching the frontend's chip labels -- codes
/// here once broke chip matching (bug: "stuck at 3" with none actually
/// picked). Empty if unconfirmed, deliberately no fallback guess.
pub fn default_classes(ing: &crate::ingest::Ingest, name: &str) -> Vec<String> {
    let Some(sym) = ing.store.names.get(name) else {
        return Vec::new();
    };
    let (resolved, _) = ing.classes.visits_by_resolved_configuration(sym.0);
    // why: the most RECENT loadout, not the most-played one -- see
    // combat::latest_visit_ms. `visits_by_resolved_configuration` orders by
    // visit count, which on a long log answers "what did you main once".
    resolved
        .into_iter()
        .max_by_key(|(_, visits)| crate::combat::latest_visit_ms(ing, visits))
        .map(|(classes, _)| classes)
        .unwrap_or_default()
}

/// why: `item.classes` is a plain code list, ["ALL"], or ["ALL_EXCEPT",
/// codes...]; the only place that sentinel gets interpreted
fn usable_by(item: &Item, active: &[String]) -> bool {
    if active.is_empty() {
        return true; // why: no filter selected, show everything
    }
    if item.classes.first().map(String::as_str) == Some("ALL") {
        return true;
    }
    if item.classes.first().map(String::as_str) == Some("ALL_EXCEPT") {
        return !active
            .iter()
            .any(|c| item.classes[1..].iter().any(|x| x == c));
    }
    active.iter().any(|c| item.classes.iter().any(|x| x == c))
}

/// why: expands ALL/ALL_EXCEPT sentinels to a concrete code list --
/// intersecting a literal ["ALL"] with plain list intersection would wrongly produce empty
fn expand_classes(classes: &[String]) -> Vec<String> {
    match classes.first().map(String::as_str) {
        Some("ALL") | None => CLASS_NAMES.iter().map(|(c, _)| c.to_string()).collect(),
        Some("ALL_EXCEPT") => CLASS_NAMES
            .iter()
            .map(|(c, _)| c.to_string())
            .filter(|c| !classes[1..].iter().any(|x| x == c))
            .collect(),
        _ => classes.to_vec(),
    }
}

/// why: real intersection, both sides pre-expanded; empty means the swap
/// leaves nothing able to wear the item, illegal not just narrowed
fn intersect_str(a: &[String], b: &[String]) -> Vec<String> {
    a.iter().filter(|x| b.contains(x)).cloned().collect()
}

/// why: item's class/slot lists narrowed by every exaltation except
/// `exclude_key`, mirrors the standalone planner's `effective()`
fn effective_classes_slots(
    item: &Item,
    assignments: &HashMap<String, String>,
    exclude_key: Option<&str>,
) -> (Vec<String>, Vec<String>) {
    let mut classes = expand_classes(&item.classes);
    let mut slots = item.slots.clone();
    for (key, source_id) in assignments {
        if exclude_key == Some(key.as_str()) {
            continue;
        }
        let Some(src) = itemdata::by_id(source_id) else {
            continue;
        };
        classes = intersect_str(&classes, &expand_classes(&src.classes));
        slots = intersect_str(&slots, &src.slots);
    }
    (classes, slots)
}

/// why: a summoned item's effect can't be extracted; both TEMPORARY tag
/// and "Summoned" name-prefix needed, neither alone catches every real case
fn is_summoned(item: &Item) -> bool {
    item.tags.iter().any(|t| t == "TEMPORARY") || item.name.to_lowercase().starts_with("summoned")
}

fn race_ok(item: &Item, race: Option<&str>) -> bool {
    let Some(race) = race else { return true };
    item.races.is_empty() || item.races.iter().any(|r| r == "ALL" || r == race)
}

/// why: wiki's "doesn't actually exist here" marker -- era filtering
/// alone can't catch this, an unimplemented item with no Era category
/// would sail through as "current" otherwise
fn on_server(item: &Item) -> bool {
    !item.categories.iter().any(|c| c == "Non-P99 Content")
}

/// why: single toggle, flip once Imbue spells go live, no per-item changes needed
const IMBUE_SPELLS_LIVE: bool = false;

fn imbue_ok(item: &Item) -> bool {
    IMBUE_SPELLS_LIVE || !item.requires_imbue
}

/// why: LORE and LORE_EQUIPPED both mean "no second copy equipped",
/// enforced per item name -- see `recommend`'s `claimed_lore`
fn is_lore(item: &Item) -> bool {
    item.tags
        .iter()
        .any(|t| t == "LORE" || t == "LORE_EQUIPPED")
}

// ---------------------------------------------------------------- era

/// why: chronological by live-EQ dates, direct port of the standalone planner's ERA_ORDER
pub const ERA_ORDER: &[&str] = &[
    "Classic Era",
    "Fear Era",
    "Hate Era",
    "Paineel Era",
    "Temple Era",
    "Sky Era",
    "Kunark Era",
    "Epic Quests Era",
    "Nov 2000 Era",
    "FearHateRevamp Era",
    "Velious Era",
    "Chardok Revamp Era",
];

/// why: where the live server actually is, a stated fact not derived from the scrape
pub const CURRENT_ERA: &str = "Sky Era";

/// why: pub so other tabs can rank an era the same way -- Epic Quests
/// asks it about the MOB an item drops from, not the item
pub fn era_ix(era: &str) -> Option<usize> {
    ERA_ORDER.iter().position(|e| *e == era)
}

/// why: earliest era, None if genuinely unresolvable; prefers
/// available_from, then eras minimum, then era. Simpler than the
/// standalone planner's full chain (no override list, no zone-voting
/// fallback) -- unresolved is left None and always shown, same stance.
pub fn era_index(item: &Item) -> Option<usize> {
    if let Some(af) = &item.available_from {
        if let Some(ix) = era_ix(af) {
            return Some(ix);
        }
    }
    if !item.eras.is_empty() {
        if let Some(ix) = item.eras.iter().filter_map(|e| era_ix(e)).min() {
            return Some(ix);
        }
    }
    item.era.as_deref().and_then(era_ix)
}

/// why: None defaults to CURRENT_ERA, matching preferences' own default.
/// Some("All") is an explicit bypass checked before lookup, not a fallback
/// from an unrecognized string. An unresolved item's era always passes.
fn in_era(item: &Item, max_era: Option<&str>) -> bool {
    let max_era = max_era.unwrap_or(CURRENT_ERA);
    if max_era == "All" {
        return true;
    }
    let Some(max_ix) = era_ix(max_era) else {
        return true;
    };
    match era_index(item) {
        Some(ix) => ix <= max_ix,
        None => true,
    }
}

/// why: slot key -> item.slots token; ANY1/ANY2 accept anything with at least one real equip slot
fn fits_slot(item: &Item, slot_key: &str) -> bool {
    let token = match slot_key {
        "EAR1" | "EAR2" => "EAR",
        "WRIST1" | "WRIST2" => "WRIST",
        "FINGER1" | "FINGER2" => "FINGERS",
        "ANY1" | "ANY2" => return !item.slots.is_empty(),
        other => other,
    };
    item.slots.iter().any(|s| s == token)
}

// ---------------------------------------------------------------- scoring

const MELEE: &[&str] = &[
    "WAR", "PAL", "SHD", "RNG", "MNK", "ROG", "BRD", "BER", "BST",
];

const W_AC_MELEE: f64 = 4.0;
const W_AC_OTHER: f64 = 3.0;
const W_HP_MELEE: f64 = 0.4;
const W_HP_HYBRID: f64 = 0.3;
const W_HP_CASTER: f64 = 0.2;
const W_MANA_BASE: f64 = 0.05;
const W_MANA_PER_CASTER: f64 = 0.05;
const W_RATIO_MELEE: f64 = 200.0;
const W_EFFECT: f64 = 4.0;

/// why: opinionated stat priorities, carried over as-is from the standalone
/// planner -- a heuristic judgement call, not verified game data
fn stat_weights(code: &str) -> &'static [(&'static str, f64)] {
    match code {
        "WAR" => &[
            ("AC", 2.0),
            ("HP", 1.0),
            ("STA", 1.5),
            ("STR", 1.2),
            ("AGI", 0.8),
            ("DEX", 0.6),
        ],
        "CLR" => &[
            ("WIS", 2.0),
            ("MANA", 1.0),
            ("AC", 1.0),
            ("HP", 0.8),
            ("STA", 0.8),
        ],
        "PAL" => &[
            ("STR", 1.2),
            ("STA", 1.2),
            ("AC", 1.5),
            ("HP", 0.9),
            ("WIS", 1.2),
            ("MANA", 0.5),
        ],
        "RNG" => &[
            ("STR", 1.2),
            ("DEX", 1.2),
            ("STA", 1.0),
            ("AC", 1.0),
            ("WIS", 1.0),
            ("MANA", 0.5),
        ],
        "SHD" => &[
            ("STR", 1.3),
            ("STA", 1.2),
            ("AC", 1.5),
            ("HP", 0.9),
            ("INT", 1.2),
            ("MANA", 0.5),
        ],
        "DRU" => &[
            ("WIS", 2.0),
            ("MANA", 1.0),
            ("AC", 0.8),
            ("HP", 0.6),
            ("STA", 0.8),
        ],
        "MNK" => &[
            ("STR", 1.3),
            ("AGI", 1.2),
            ("DEX", 1.0),
            ("STA", 1.0),
            ("AC", 1.2),
        ],
        "BRD" => &[
            ("CHA", 1.2),
            ("DEX", 1.2),
            ("INT", 1.2),
            ("MANA", 0.8),
            ("STA", 0.8),
            ("AC", 1.0),
        ],
        "ROG" => &[
            ("DEX", 1.5),
            ("AGI", 1.2),
            ("STR", 1.2),
            ("STA", 0.8),
            ("AC", 0.8),
        ],
        "SHM" => &[("WIS", 2.0), ("MANA", 1.0), ("STA", 0.8), ("AC", 0.8)],
        "NEC" => &[
            ("INT", 2.0),
            ("MANA", 1.0),
            ("STA", 0.8),
            ("AC", 0.6),
            ("HP", 0.6),
        ],
        "WIZ" => &[("INT", 2.0), ("MANA", 1.0), ("STA", 0.8), ("AC", 0.6)],
        "MAG" => &[("INT", 2.0), ("MANA", 1.0), ("STA", 0.8), ("AC", 0.6)],
        "ENC" => &[
            ("INT", 2.0),
            ("MANA", 1.0),
            ("CHA", 0.8),
            ("STA", 0.8),
            ("AC", 0.6),
        ],
        "BST" => &[
            ("WIS", 1.5),
            ("MANA", 0.8),
            ("STR", 1.0),
            ("STA", 1.0),
            ("AGI", 0.8),
            ("AC", 1.0),
        ],
        "BER" => &[
            ("STR", 1.5),
            ("STA", 1.2),
            ("DEX", 1.0),
            ("AC", 1.0),
            ("HP", 0.8),
        ],
        _ => &[],
    }
}

fn uses_mana(code: &str) -> bool {
    stat_weights(code)
        .iter()
        .any(|(k, v)| *k == "MANA" && *v > 0.0)
}

/// why: read straight off stat_weights so it can't drift; None for a
/// no-mana class. pub(crate) so `crate::character` shares this lookup too.
pub(crate) fn casting_stat(code: &str) -> Option<&'static str> {
    if !uses_mana(code) {
        return None;
    }
    stat_weights(code)
        .iter()
        .find_map(|&(k, v)| (v > 0.0 && (k == "INT" || k == "WIS")).then_some(k))
}

/// why: real formula lives in `crate::manadata` now, superseded three
/// earlier reverse-engineered approximations here. What's left is a
/// scoring helper -- marginal value of one more INT/WIS point for
/// ranking gear, not a pool reconstruction.
fn mana_marginal_rate(active: &[String], level: u8) -> f64 {
    // why: fixed reference stat, not a real total -- a relative weight
    // for ranking, measured at the same point for every class
    const REFERENCE_STAT: f64 = 150.0;
    let rates: Vec<f64> = active
        .iter()
        .filter_map(|c| {
            let hi = crate::manadata::class_mana_pool(c, level, REFERENCE_STAT + 1.0)?;
            let lo = crate::manadata::class_mana_pool(c, level, REFERENCE_STAT)?;
            Some(hi - lo)
        })
        .collect();
    if rates.is_empty() {
        0.0
    } else {
        rates.iter().sum::<f64>() / rates.len() as f64
    }
}

/// why: class-derived weight vector. AC/HP/MANA are the mean across
/// active classes; most stats are the max; INT/WIS are the exception,
/// once `level` is known -- a flat max-based weight can't represent the
/// real mana mechanic (top-2-of-3 classes' pools summed), so growing a
/// shared stat helps two pool-slots at once. Multiplier capped at 2:
/// one class on a stat = 1, majority-shared stat = 2 (minority gets 0,
/// since this app can't know if it lands in the real top two), equal
/// split = 1 each. All resolve through the same mana_per_point number
/// so +MANA and INT/WIS stay directly comparable, no double-counting.
fn derived_weights(active: &[String], level: Option<u8>) -> HashMap<String, f64> {
    let mut w: HashMap<String, f64> = HashMap::new();
    for &k in &[
        "AC", "HP", "MANA", "STR", "STA", "AGI", "DEX", "WIS", "INT", "CHA",
    ] {
        w.insert(k.to_string(), 0.0);
    }
    for c in active {
        for &(k, v) in stat_weights(c) {
            let e = w.entry(k.to_string()).or_insert(0.0);
            if v > *e {
                *e = v;
            }
        }
    }
    if !active.is_empty() {
        let n = active.len() as f64;
        let mean = |f: fn(&str) -> f64| active.iter().map(|c| f(c)).sum::<f64>() / n;
        w.insert(
            "AC".into(),
            mean(|c| {
                if MELEE.contains(&c) {
                    W_AC_MELEE
                } else {
                    W_AC_OTHER
                }
            }),
        );
        w.insert(
            "HP".into(),
            mean(|c| {
                if !MELEE.contains(&c) {
                    W_HP_CASTER
                } else if uses_mana(c) {
                    W_HP_HYBRID
                } else {
                    W_HP_MELEE
                }
            }),
        );
        // why: pool_slots caps redundant same-stat casters at 2, not 3
        let n_int = active
            .iter()
            .filter(|c| casting_stat(c) == Some("INT"))
            .count();
        let n_wis = active
            .iter()
            .filter(|c| casting_stat(c) == Some("WIS"))
            .count();
        let (mult_int, mult_wis): (f64, f64) = match (n_int, n_wis) {
            (0, 0) => (0.0, 0.0),
            (i, 0) => (i.min(2) as f64, 0.0),
            (0, wi) => (0.0, wi.min(2) as f64),
            (i, wi) if i == wi => (i as f64, wi as f64),
            (i, wi) if i > wi => (i.min(2) as f64, 0.0),
            (_, wi) => (0.0, wi.min(2) as f64),
        };
        let pool_slots = mult_int + mult_wis;
        let mana_per_point = (W_MANA_BASE + W_MANA_PER_CASTER * pool_slots).min(0.2);
        w.insert("MANA".into(), mana_per_point);

        if let Some(level) = level {
            let k = mana_marginal_rate(active, level);
            w.insert("INT".into(), mana_per_point * k * mult_int);
            w.insert("WIS".into(), mana_per_point * k * mult_wis);
        }
    }
    w.insert(
        "RATIO".into(),
        if active.iter().any(|c| MELEE.contains(&c.as_str())) {
            W_RATIO_MELEE
        } else {
            0.0
        },
    );
    w.insert("EFFECT".into(), W_EFFECT);
    w
}

const SCORED_EFFECTS: &[&str] = &["focus", "click", "worn"];

/// why: dot product of stats against weights, plus damage/delay ratio and
/// a flat bonus per scored effect (proc excluded -- graftable via
/// exaltation, scoring it directly could let a mediocre item win on it).
/// `tier` scales before scoring, so an owned +8 item scores as +8, not fresh.
fn score_item(item: &Item, weights: &HashMap<String, f64>, tier: u8) -> f64 {
    let mut s = 0.0;
    // why: sorted, because float addition is not associative -- walking
    // `stats` in HashMap order made the same item score
    // 32.33333333333333 one run and 32.333333333333336 the next
    let mut stats: Vec<_> = item.stats.iter().collect();
    stats.sort_by(|a, b| a.0.cmp(b.0));
    for (stat, val) in stats {
        if let Some(w) = weights.get(stat.as_str()) {
            s += w * scale_stat(*val, tier);
        }
    }
    if let (Some(dmg), Some(delay)) = (item.dmg, item.delay) {
        if delay > 0.0 {
            s += weights.get("RATIO").copied().unwrap_or(0.0) * (scale_stat(dmg, tier) / delay);
        }
    }
    let effect_count = SCORED_EFFECTS
        .iter()
        .filter(|e| item.effects.contains_key(**e))
        .count() as f64;
    s += weights.get("EFFECT").copied().unwrap_or(0.0) * effect_count;
    s
}

/// why: skill prefix "2H" marks a weapon two-handed
fn is_two_hand(item: &Item) -> bool {
    item.skill.as_deref().is_some_and(|s| s.starts_with("2H"))
}

// ---------------------------------------------------------------- item tiers

/// why: +10% of base per tier, min +1/tier, whichever is larger -- ported
/// from the standalone planner, re-confirmed. Rounds not floors -- a real
/// item (Tobrin's Mystical Eyepatch, INT 15->17 at +1) caught the source's
/// own floor as wrong on a half-point the AC-30 wiki example never hit.
/// Negative branch is the formula taken literally, not independently confirmed.
fn scale_stat(base: f64, tier: u8) -> f64 {
    if base == 0.0 || tier == 0 {
        return base;
    }
    let t = tier as f64;
    if base > 0.0 {
        (base * (1.0 + t / 10.0)).round().max(base + t)
    } else {
        -((-base) * (1.0 + t / 10.0)).round()
    }
}

/// why: weight shrinks as an item tempers up, same fraction scale_stat
/// grows by, floored at 0.1 so it never reaches zero
fn scale_weight(base: f64, tier: u8) -> f64 {
    if tier == 0 {
        return base;
    }
    let shrunk = base * (1.0 - tier as f64 / 10.0);
    ((shrunk * 10.0).round() / 10.0).max(0.1)
}

/// why: native effect narrowed to what the preview panel needs; missing
/// name treated as absent, not a panic -- scraped data, shape not controlled here
#[derive(Debug, Clone, Serialize)]
pub struct ItemEffectDto {
    pub name: String,
    pub detail: Option<String>,
}

fn item_effect(item: &Item, key: &str) -> Option<ItemEffectDto> {
    let v = item.effects.get(key)?;
    Some(ItemEffectDto {
        name: v.get("name")?.as_str()?.to_string(),
        detail: v.get("detail").and_then(|d| d.as_str()).map(String::from),
    })
}

/// why: exaltation socket family in unlock order, ported from the
/// standalone planner; Ornamentation has no effect_key, takes a cosmetic token
const EXALT_SLOTS: &[(&str, &str, u8, Option<&str>)] = &[
    ("ornament", "Ornamentation", 0, None),
    ("focus", "Focus", 1, Some("focus")),
    ("click", "Click", 2, Some("click")),
    ("worn", "Worn", 3, Some("worn")),
    ("proc", "Proc", 4, Some("proc")),
];

/// why: one socket's unlock state + native effect only; grafting a
/// different item's effect (real "exaltation") isn't attempted here yet
#[derive(Debug, Clone, Serialize)]
pub struct ExaltSlotDto {
    pub key: String,
    pub label: String,
    pub req_tier: u8,
    pub unlocked: bool,
    pub effect: Option<ItemEffectDto>,
}

fn exalt_slots(item: &Item, tier: u8) -> Vec<ExaltSlotDto> {
    EXALT_SLOTS
        .iter()
        .map(|&(key, label, req_tier, effect_key)| {
            let unlocked = tier >= req_tier;
            ExaltSlotDto {
                key: key.to_string(),
                label: label.to_string(),
                req_tier,
                unlocked,
                effect: if unlocked {
                    effect_key.and_then(|k| item_effect(item, k))
                } else {
                    None
                },
            }
        })
        .collect()
}

// ---------------------------------------------------------------- DTOs + commands

#[derive(Debug, Clone, Serialize)]
pub struct ItemDto {
    pub id: String,
    pub name: String,
    pub tags: Vec<String>,
    pub slots: Vec<String>,
    pub classes: Vec<String>,
    pub stats: HashMap<String, f64>,
    pub dmg: Option<f64>,
    pub delay: Option<f64>,
    pub skill: Option<String>,
    pub era: Option<String>,
    pub icon: Option<String>,
    /// why: first drop source, "zone — mob" shape, mob drop takes priority
    pub source: Option<String>,
    /// why: every zone deduplicated; source only surfaces the first, a drop can span several
    pub zones: Vec<String>,
    /// why: every mob across all zones, deduplicated
    pub mobs: Vec<String>,
    pub url: Option<String>,
    pub wt: Option<f64>,
    pub size: Option<String>,
    /// why: "+N" this instance is at, 0 for a browse/recommend; stats/dmg
    /// already scaled to this, carried alongside for display only
    pub tier: u8,
    /// why: total copies owned anywhere; never same-slot-only, tells
    /// "own one, need a second" from "own none"
    pub owned: u32,
    /// why: native effects, tier-independent -- upgrading only unlocks the socket, see exalts
    pub effects: HashMap<String, ItemEffectDto>,
    /// why: the 5 exaltation sockets at this tier, see `exalt_slots`
    pub exalts: Vec<ExaltSlotDto>,
    /// why: real-session proc evidence, only ever filled by `resolve_inventory`
    pub proc_evidence: Option<ProcEvidenceDto>,
}

/// why: the only per-item exaltation fact this app can confirm -- never
/// which effect, only that the socket is live and how many times it fired
#[derive(Debug, Clone, Serialize)]
pub struct ProcEvidenceDto {
    pub fires: u32,
    pub first_seen_ms: Millis,
}

/// why: first-occurrence dedup, order preserved; small lists, a HashSet filter is plenty
fn dedup_keep_order(items: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    items
        .into_iter()
        .filter(|x| seen.insert(x.clone()))
        .collect()
}

fn source_label(item: &Item) -> Option<String> {
    if let Some(d) = item.drops.first() {
        return Some(match d.mobs.first() {
            Some(m) => format!("{} — {}", d.zone, m),
            None => d.zone.clone(),
        });
    }
    if let Some(q) = item.quests.first() {
        return Some(format!("quest: {q}"));
    }
    if let Some(v) = item.vendors.first() {
        return Some(format!("vendor: {v}"));
    }
    if !item.crafted_by.is_empty() {
        return Some("crafted".to_string());
    }
    None
}

/// why: tier 0 for every caller except resolve_inventory -- a no-op at 0, old base-stats behavior
fn to_dto(item: &Item, tier: u8, owned: u32) -> ItemDto {
    ItemDto {
        id: item.id.clone(),
        name: item.name.clone(),
        tags: item.tags.clone(),
        slots: item.slots.clone(),
        classes: item
            .classes
            .iter()
            .filter_map(|c| {
                code_to_name(c)
                    .map(String::from)
                    .or_else(|| Some(c.clone()))
            })
            .collect(),
        stats: item
            .stats
            .iter()
            .map(|(k, &v)| (k.clone(), scale_stat(v, tier)))
            .collect(),
        dmg: item.dmg.map(|d| scale_stat(d, tier)),
        delay: item.delay,
        skill: item.skill.clone(),
        era: item.era.clone(),
        icon: item.icon.clone(),
        source: source_label(item),
        zones: dedup_keep_order(item.drops.iter().map(|d| d.zone.clone())),
        mobs: dedup_keep_order(item.drops.iter().flat_map(|d| d.mobs.iter().cloned())),
        url: item.url.clone(),
        wt: item.wt.map(|w| scale_weight(w, tier)),
        size: item.size.clone(),
        tier,
        owned,
        effects: EXALT_SLOTS
            .iter()
            .filter_map(|&(_, _, _, effect_key)| effect_key)
            .filter_map(|k| item_effect(item, k).map(|e| (k.to_string(), e)))
            .collect(),
        exalts: exalt_slots(item, tier),
        proc_evidence: None,
    }
}

/// why: "level this up and show me" what-if preview; None for a bad id.
/// Always owned: 0 -- this has no dump context, only refreshes tier-dependent fields.
pub fn item_at_tier(id: &str, tier: u8) -> Option<ItemDto> {
    let item = itemdata::by_id(id)?;
    Some(to_dto(item, tier.min(10), 0))
}

/// why: shows the assigned source's effect instead of the item's native
/// one, what actually happens in-game; separate fn so plain native-effect callers stay untouched
fn exalt_slots_with_assignments(
    item: &Item,
    tier: u8,
    assignments: &HashMap<String, String>,
) -> Vec<ExaltSlotDto> {
    EXALT_SLOTS
        .iter()
        .map(|&(key, label, req_tier, effect_key)| {
            let unlocked = tier >= req_tier;
            let effect = if !unlocked {
                None
            } else if let Some(source_id) = assignments.get(key) {
                itemdata::by_id(source_id)
                    .and_then(|src| effect_key.and_then(|k| item_effect(src, k)))
            } else {
                effect_key.and_then(|k| item_effect(item, k))
            };
            ExaltSlotDto {
                key: key.to_string(),
                label: label.to_string(),
                req_tier,
                unlocked,
                effect,
            }
        })
        .collect()
}

/// why: item_at_tier's DTO with exalts re-derived against "what if" assignments; other fields unaffected
pub fn item_with_exalts(
    id: &str,
    tier: u8,
    assignments: &HashMap<String, String>,
) -> Option<ItemDto> {
    let item = itemdata::by_id(id)?;
    let tier = tier.min(10);
    let mut dto = to_dto(item, tier, 0);
    dto.exalts = exalt_slots_with_assignments(item, tier, assignments);
    Some(dto)
}

/// why: legal exaltation sources for a socket, mirrors the standalone
/// planner's legalSources -- must carry the matching effect and leave a
/// real class/slot intersection, or it isn't offered at all. `classes`
/// filters by the player's active trio; empty means no filter. Excludes
/// the item itself and every summoned/out-of-era/off-server source.
pub fn exalt_candidates(
    item_id: &str,
    socket_key: &str,
    other_assignments: &HashMap<String, String>,
    classes: &[String],
    max_era: Option<&str>,
) -> Vec<ItemDto> {
    let Some(item) = itemdata::by_id(item_id) else {
        return Vec::new();
    };
    let Some(&(_, _, _, Some(effect_key))) = EXALT_SLOTS.iter().find(|&&(k, ..)| k == socket_key)
    else {
        return Vec::new(); // why: unknown socket, or ornament -- nothing to source
    };
    let (eff_classes, eff_slots) =
        effective_classes_slots(item, other_assignments, Some(socket_key));
    let codes = names_to_codes(classes);
    itemdata::items()
        .iter()
        .filter(|src| src.id != item.id)
        .filter(|src| src.effects.contains_key(effect_key))
        .filter(|src| !is_summoned(src))
        .filter(|src| on_server(src))
        .filter(|src| in_era(src, max_era))
        .filter(|src| usable_by(src, &codes))
        .filter(|src| {
            let new_classes = intersect_str(&eff_classes, &expand_classes(&src.classes));
            let new_slots = intersect_str(&eff_slots, &src.slots);
            !new_classes.is_empty() && !new_slots.is_empty()
        })
        .map(|src| to_dto(src, 0, 0))
        .collect()
}

#[derive(Debug, Clone, Serialize)]
pub struct InventoryDumpDto {
    /// why: matched item, full DTO, same shape a recommendation's items carry
    pub resolved: HashMap<String, ItemDto>,
    /// why: dump's own printed name for an unmatched item, surfaced not silently dropped
    pub unresolved: HashMap<String, String>,
    /// why: total copies owned, passed through for items beyond what's equipped
    pub owned: HashMap<String, u32>,
    /// why: highest tier owned, so recommend/list_items score at the real tier not +0
    pub owned_tier: HashMap<String, u8>,
}

/// why: matches a parsed dump against the catalog by exact name
/// (case-insensitive); the one place stats get scaled to what's
/// actually equipped, not wiki-scraped base. Converts exalted[slot]'s
/// socket key -> source name into socket key -> source id; an
/// unresolvable name is dropped, falling back to the native effect.
fn resolve_exalt_assignments(
    exalted_for_slot: &HashMap<String, String>,
) -> HashMap<String, String> {
    exalted_for_slot
        .iter()
        .filter_map(|(socket_key, source_name)| {
            let src = itemdata::items()
                .iter()
                .find(|it| it.name.eq_ignore_ascii_case(source_name))?;
            Some((socket_key.clone(), src.id.clone()))
        })
        .collect()
}

/// why: None for a caller with no live session; stays optional so a bare test doesn't need one
pub fn resolve_inventory(
    parsed: &crate::inventory::ParsedInventory,
    proc_evidence: Option<&ExaltationProcs>,
) -> InventoryDumpDto {
    let mut dto = InventoryDumpDto {
        resolved: HashMap::new(),
        unresolved: HashMap::new(),
        owned: parsed.owned.clone(),
        owned_tier: parsed.owned_tier.clone(),
    };
    for (slot, inv_item) in &parsed.equipped {
        match itemdata::items()
            .iter()
            .find(|it| it.name.eq_ignore_ascii_case(&inv_item.name))
        {
            Some(it) => {
                let owned = parsed.owned.get(&it.name).copied().unwrap_or(0);
                let mut item_dto = to_dto(it, inv_item.tier, owned);
                // why: the dump's real socketed exaltations beat native
                // effects -- real ground truth; empty falls back unaffected
                if let Some(exalted) = parsed.exalted.get(slot) {
                    let assignments = resolve_exalt_assignments(exalted);
                    if !assignments.is_empty() {
                        item_dto.exalts =
                            exalt_slots_with_assignments(it, inv_item.tier, &assignments);
                    }
                }
                if let Some(procs) = proc_evidence {
                    let fires = procs.count(&it.name);
                    if fires > 0 {
                        item_dto.proc_evidence = Some(ProcEvidenceDto {
                            fires,
                            first_seen_ms: procs.first_seen_ms(&it.name).unwrap_or(0),
                        });
                    }
                }
                dto.resolved.insert(slot.clone(), item_dto);
            }
            None => {
                dto.unresolved.insert(slot.clone(), inv_item.name.clone());
            }
        }
    }
    dto
}

/// why: every usable item, filtered by class/race/era, optionally one slot; empty classes = unfiltered
pub fn list_items(
    classes: &[String],
    slot: Option<&str>,
    max_era: Option<&str>,
    owned: Option<&HashMap<String, u32>>,
    owned_tier: Option<&HashMap<String, u8>>,
) -> Vec<ItemDto> {
    let codes = names_to_codes(classes);
    itemdata::items()
        .iter()
        .filter(|it| usable_by(it, &codes))
        .filter(|it| slot.is_none_or(|s| fits_slot(it, s)))
        .filter(|it| in_era(it, max_era))
        .filter(|it| on_server(it))
        .filter(|it| imbue_ok(it))
        .map(|it| {
            to_dto(
                it,
                owned_tier
                    .and_then(|o| o.get(&it.name))
                    .copied()
                    .unwrap_or(0),
                owned.and_then(|o| o.get(&it.name)).copied().unwrap_or(0),
            )
        })
        .collect()
}

#[derive(Debug, Clone, Serialize)]
pub struct SlotRecommendationDto {
    pub slot: String,
    pub items: Vec<ScoredItemDto>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScoredItemDto {
    #[serde(flatten)]
    pub item: ItemDto,
    pub score: f64,
}

pub const SLOTS: &[(&str, &str)] = &[
    ("EAR1", "Ear"),
    ("HEAD", "Head"),
    ("FACE", "Face"),
    ("EAR2", "Ear"),
    ("NECK", "Neck"),
    ("SHOULDERS", "Shoulders"),
    ("ARMS", "Arms"),
    ("BACK", "Back"),
    ("WRIST1", "Wrist"),
    ("WRIST2", "Wrist"),
    ("RANGE", "Range"),
    ("HANDS", "Hands"),
    ("PRIMARY", "Primary"),
    ("SECONDARY", "Secondary"),
    ("FINGER1", "Finger"),
    ("FINGER2", "Finger"),
    ("CHEST", "Chest"),
    ("LEGS", "Legs"),
    ("FEET", "Feet"),
    ("WAIST", "Waist"),
    ("AMMO", "Ammo"),
    ("ANY1", "Any"),
    ("ANY2", "Any"),
];

/// why: top candidates per slot, scored against derived or custom
/// weights. LORE duplicates handled via `claimed_lore` -- once placed, a
/// name is removed from every other slot's pool. `equipped` seeds
/// claimed_lore with what's actually worn; `owned`/`owned_tier` feed
/// display and score at the real owned tier, not a fresh +0.
///
/// why: nets 2H Primary score against best forfeited Secondary (melee only)
#[allow(clippy::too_many_arguments)] // each param is its own real, independently-optional filter -- see doc above
pub fn recommend(
    classes: &[String],
    race: Option<&str>,
    max_era: Option<&str>,
    per_slot: usize,
    custom_weights: Option<HashMap<String, f64>>,
    level: Option<u8>,
    equipped: Option<&HashMap<String, String>>,
    owned: Option<&HashMap<String, u32>>,
    owned_tier: Option<&HashMap<String, u8>>,
) -> Vec<SlotRecommendationDto> {
    let codes = names_to_codes(classes);
    let weights = custom_weights.unwrap_or_else(|| derived_weights(&codes, level));
    let race_code = race.and_then(race_name_to_code);
    let owned_of = |name: &str| owned.and_then(|o| o.get(name)).copied().unwrap_or(0);
    let tier_of = |name: &str| owned_tier.and_then(|o| o.get(name)).copied().unwrap_or(0);

    let ratio_weight = weights.get("RATIO").copied().unwrap_or(0.0);
    let best_secondary_score = if ratio_weight > 0.0 {
        itemdata::items()
            .iter()
            .filter(|it| fits_slot(it, "SECONDARY"))
            .filter(|it| usable_by(it, &codes))
            .filter(|it| race_ok(it, race_code))
            .filter(|it| in_era(it, max_era))
            .filter(|it| on_server(it))
            .filter(|it| imbue_ok(it))
            .map(|it| score_item(it, &weights, tier_of(&it.name)))
            .fold(0.0_f64, f64::max)
    } else {
        0.0
    };

    // why: name -> the one slot allowed to claim it (names not ids, the
    // in-game restriction is on the name). Seeded from what's really
    // equipped, then filled by each slot's own greedy top pick in SLOTS
    // order -- ANY slots last, so a dedicated slot claims a LORE item first.
    let mut claimed_lore: HashMap<&str, &str> = HashMap::new();
    if let Some(eq) = equipped {
        for (slot_key, name) in eq {
            if let Some(catalog) = itemdata::items()
                .iter()
                .find(|it| it.name.eq_ignore_ascii_case(name))
            {
                if is_lore(catalog) {
                    claimed_lore.insert(catalog.name.as_str(), slot_key.as_str());
                }
            }
        }
    }

    SLOTS
        .iter()
        .map(|(key, _)| {
            let mut scored: Vec<(f64, &Item)> = itemdata::items()
                .iter()
                .filter(|it| fits_slot(it, key))
                .filter(|it| usable_by(it, &codes))
                .filter(|it| race_ok(it, race_code))
                .filter(|it| in_era(it, max_era))
                .filter(|it| on_server(it))
                .filter(|it| imbue_ok(it))
                .filter(|it| {
                    !is_lore(it)
                        || claimed_lore
                            .get(it.name.as_str())
                            .is_none_or(|owner| *owner == *key)
                })
                .map(|it| {
                    let raw = score_item(it, &weights, tier_of(&it.name));
                    let net = if *key == "PRIMARY" && is_two_hand(it) {
                        raw - best_secondary_score
                    } else {
                        raw
                    };
                    (net, it)
                })
                .collect();
            scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
            scored.truncate(per_slot);
            // why: only the top pick claims the name -- an alt further down isn't worn
            if let Some((_, top)) = scored.first() {
                if is_lore(top) {
                    claimed_lore.entry(top.name.as_str()).or_insert(key);
                }
            }
            SlotRecommendationDto {
                slot: key.to_string(),
                items: scored
                    .into_iter()
                    .map(|(score, it)| ScoredItemDto {
                        item: to_dto(it, tier_of(&it.name), owned_of(&it.name)),
                        score,
                    })
                    .collect(),
            }
        })
        .collect()
}

/// why: the scoring vector itself, so the UI can show why items rank the way they do
pub fn weights_for(classes: &[String], level: Option<u8>) -> HashMap<String, f64> {
    derived_weights(&names_to_codes(classes), level)
}

#[cfg(test)]
mod scale_stat_tests {
    use super::*;

    /// why: wiki's own worked example, Umbral Platemail Breastplate AC 30
    #[test]
    fn matches_the_wiki_worked_example() {
        assert_eq!(scale_stat(30.0, 1), 33.0);
        assert_eq!(scale_stat(30.0, 2), 36.0);
    }

    #[test]
    fn ten_percent_wins_when_it_beats_the_per_tier_floor() {
        // why: floor(100*1.1)=110 vs +1/tier floor of 101 -- 10% wins
        assert_eq!(scale_stat(100.0, 1), 110.0);
    }

    #[test]
    fn per_tier_floor_wins_when_ten_percent_rounds_too_low() {
        // why: round(3.3)=3, 10% rounds to nothing, but +1/tier floor guarantees +1 -- 4 wins
        assert_eq!(scale_stat(3.0, 1), 4.0);
    }

    /// why: real item, INT 15 reads 17 at +1 not 16 -- caught scale_stat's old floor bug
    #[test]
    fn ties_round_up_not_down() {
        assert_eq!(scale_stat(15.0, 1), 17.0);
    }

    #[test]
    fn plus_ten_is_doubling_or_adding_ten_whichever_is_more() {
        assert_eq!(
            scale_stat(30.0, 10),
            60.0,
            "10% x 10 tiers doubles a stat large enough for that to win"
        );
        assert_eq!(
            scale_stat(5.0, 10),
            15.0,
            "too small for doubling to beat +10 -- the flat floor wins instead"
        );
    }

    #[test]
    fn zero_and_untiered_are_left_alone() {
        assert_eq!(scale_stat(0.0, 10), 0.0);
        assert_eq!(scale_stat(42.0, 0), 42.0);
    }

    #[test]
    fn negative_stats_scale_the_same_percentage_without_a_floor() {
        // why: no wiki example for negative stats -- pins the literal formula reading
        assert_eq!(scale_stat(-20.0, 2), -24.0);
    }

    /// why: end to end through resolve_inventory, not the isolated function
    #[test]
    fn resolve_inventory_scales_a_real_catalog_item() {
        let mut equipped = HashMap::new();
        equipped.insert(
            "NECK".to_string(),
            crate::inventory::InventoryItem {
                name: "A Bone Necklace".to_string(),
                tier: 5,
            },
        );
        let parsed = crate::inventory::ParsedInventory {
            equipped,
            ..Default::default()
        };
        let dto = resolve_inventory(&parsed, None);
        let item = dto
            .resolved
            .get("NECK")
            .expect("A Bone Necklace is a real catalog item");
        assert_eq!(item.tier, 5);
        assert_eq!(item.stats.get("AC"), Some(&7.0));
    }

    /// why: real dump data, item has no native focus -- socketed effect must come from the dump
    #[test]
    fn resolve_inventory_shows_the_dumps_own_real_socketed_exaltation() {
        let mut equipped = HashMap::new();
        equipped.insert(
            "BACK".to_string(),
            crate::inventory::InventoryItem {
                name: "Shield of the Immaculate".to_string(),
                tier: 3,
            },
        );
        let mut back_sockets = HashMap::new();
        back_sockets.insert("focus".to_string(), "White Dragonscale Cloak".to_string());
        let mut exalted = HashMap::new();
        exalted.insert("BACK".to_string(), back_sockets);
        let parsed = crate::inventory::ParsedInventory {
            equipped,
            exalted,
            ..Default::default()
        };
        let dto = resolve_inventory(&parsed, None);
        let item = dto
            .resolved
            .get("BACK")
            .expect("Shield of the Immaculate is a real catalog item");
        let focus = item
            .exalts
            .iter()
            .find(|e| e.key == "focus")
            .expect("focus socket");
        assert_eq!(focus.effect.as_ref().map(|e| e.name.as_str()), Some("Improved Damage III"), "the socketed source's own effect, not Shield of the Immaculate's own (it has no native focus effect at all)");
    }
}

#[cfg(test)]
mod two_hand_tradeoff_tests {
    use super::*;

    /// why: 2H PRIMARY score must net out best SECONDARY, real catalog
    #[test]
    fn a_2h_primarys_score_is_netted_against_the_best_secondary_real_catalog() {
        let classes = vec!["Warrior".to_string()];
        let codes = names_to_codes(&classes);
        let weights = derived_weights(&codes, Some(50));
        assert!(
            weights.get("RATIO").copied().unwrap_or(0.0) > 0.0,
            "Warrior is melee -- RATIO should carry weight"
        );

        // why: same filter chain recommend's own precompute uses -- None
        // era means current-era ceiling not unfiltered, must match exactly
        let secondary_best = itemdata::items()
            .iter()
            .filter(|it| fits_slot(it, "SECONDARY"))
            .filter(|it| usable_by(it, &codes))
            .filter(|it| race_ok(it, None))
            .filter(|it| in_era(it, None))
            .filter(|it| on_server(it))
            .filter(|it| imbue_ok(it))
            .map(|it| score_item(it, &weights, 0))
            .fold(0.0_f64, f64::max);
        assert!(
            secondary_best > 0.0,
            "the real catalog should have at least one usable Warrior secondary item"
        );

        // why: no per_slot truncation -- a real 2H can net negative and
        // fall out of a small top-N; the netting math is under test, not ranking
        let recs = recommend(&classes, None, None, 1000, None, Some(50), None, None, None);
        let primary = recs
            .iter()
            .find(|r| r.slot == "PRIMARY")
            .expect("PRIMARY is always in SLOTS");
        let two_hander = primary
            .items
            .iter()
            .find(|scored| {
                scored
                    .item
                    .skill
                    .as_deref()
                    .is_some_and(|s| s.starts_with("2H"))
            })
            .expect(
                "the real catalog has 2H Warrior Primary weapons (confirmed in packs/items.json)",
            );

        let real_item = itemdata::items()
            .iter()
            .find(|it| it.id == two_hander.item.id)
            .expect("id round-trips to a real catalog item");
        let raw = score_item(real_item, &weights, 0);
        assert!(
            (two_hander.score - (raw - secondary_best)).abs() < 1e-9,
            "2H PRIMARY score should be raw minus the best forfeited SECONDARY"
        );
        assert!(two_hander.score < raw, "netting against a real positive SECONDARY score should always reduce the 2H's own ranking score");
    }

    /// why: opposite case -- pure caster's RATIO weight is 0, no forfeit adjustment
    #[test]
    fn a_casters_2h_score_is_left_unadjusted() {
        let classes = vec!["Wizard".to_string()];
        let codes = names_to_codes(&classes);
        let weights = derived_weights(&codes, Some(50));
        assert_eq!(
            weights.get("RATIO").copied().unwrap_or(0.0),
            0.0,
            "Wizard is a pure caster -- RATIO should be 0"
        );

        let recs = recommend(&classes, None, None, 50, None, Some(50), None, None, None);
        let primary = recs
            .iter()
            .find(|r| r.slot == "PRIMARY")
            .expect("PRIMARY is always in SLOTS");
        for scored in &primary.items {
            let real_item = itemdata::items()
                .iter()
                .find(|it| it.id == scored.item.id)
                .expect("id round-trips to a real catalog item");
            let raw = score_item(real_item, &weights, 0);
            assert!(
                (scored.score - raw).abs() < 1e-9,
                "caster PRIMARY scores (2H or not) should be unadjusted raw scores"
            );
        }
    }
}

#[cfg(test)]
mod mana_pool_slot_tests {
    use super::*;

    fn int_weight(classes: &[&str]) -> f64 {
        let classes: Vec<String> = classes.iter().map(|c| c.to_string()).collect();
        let codes = names_to_codes(&classes);
        derived_weights(&codes, Some(50))["INT"]
    }

    /// why: 3rd same-stat caster must not inflate weight past 2's cap
    #[test]
    fn a_redundant_third_same_stat_caster_does_not_inflate_the_weight() {
        let two = int_weight(&["Wizard", "Necromancer", "Warrior"]);
        let three = int_weight(&["Wizard", "Magician", "Necromancer"]);
        assert!(
            (two - three).abs() < 1e-9,
            "2 same-stat + 1 non-caster ({two}) should equal 3 same-stat ({three})"
        );
    }

    /// why: one-of-each splits the 2 slots, doesn't double one stat
    #[test]
    fn one_of_each_splits_the_two_pool_slots_instead_of_doubling_one() {
        let monopoly = int_weight(&["Wizard", "Magician", "Necromancer"]); // INT x2, WIS x0
        let split = int_weight(&["Wizard", "Cleric", "Warrior"]); // INT x1, WIS x1
        assert!(
            (split - monopoly / 2.0).abs() < 1e-9,
            "split INT weight ({split}) should be half the monopoly weight ({monopoly})"
        );
    }
}

#[cfg(test)]
mod imbue_gate_tests {
    use super::*;

    /// why: real catalog item crafted from an Imbued gem, still findable
    #[test]
    fn an_imbued_gem_item_exists_in_the_catalog_and_is_flagged() {
        let it = itemdata::items()
            .iter()
            .find(|it| it.name == "Imbued Platinum Fire Ring")
            .expect("catalog should still carry this item");
        assert!(it.requires_imbue, "scrape should flag this as imbue-gated");
    }

    /// why: imbue-gated items never reach list_items while the gate is off
    #[test]
    fn list_items_excludes_imbue_gated_items() {
        let classes = vec!["Cleric".to_string()];
        let out = list_items(&classes, Some("FINGER1"), None, None, None);
        assert!(
            !out.iter().any(|it| it.name == "Imbued Platinum Fire Ring"),
            "imbue-gated ring leaked through list_items"
        );
        // why: a real, non-gated finger item for the same class should still show
        assert!(
            !out.is_empty(),
            "gate shouldn't have emptied the whole slot"
        );
    }
}

#[cfg(test)]
mod lore_ownership_tests {
    use super::*;

    fn has_named(recs: &[SlotRecommendationDto], slot: &str, name: &str) -> bool {
        recs.iter()
            .find(|r| r.slot == slot)
            .is_some_and(|r| r.items.iter().any(|it| it.item.name == name))
    }

    /// why: a real LORE ring already worn in FINGER1 can't also fill FINGER2
    #[test]
    fn a_real_equipped_lore_item_is_not_offered_for_the_sibling_slot() {
        let classes = vec!["Warrior".to_string()];
        let mut equipped = HashMap::new();
        equipped.insert("FINGER1".to_string(), "Brass Ring".to_string());

        let recs = recommend(
            &classes,
            None,
            None,
            200,
            None,
            None,
            Some(&equipped),
            None,
            None,
        );
        assert!(
            has_named(&recs, "FINGER1", "Brass Ring"),
            "a slot's own real item should still appear in its own candidate list"
        );
        assert!(
            !has_named(&recs, "FINGER2", "Brass Ring"),
            "a LORE ring worn in FINGER1 must not also be offered for FINGER2"
        );
    }

    /// why: with nothing equipped, an alt-list *browse* may legitimately
    /// show the same LORE item in both finger slots (you haven't committed
    /// it to either yet) -- only a *real* equipped copy locks the name,
    /// per the test above. This just confirms `None` context is safe and
    /// each slot's own top pick isn't spuriously filtered.
    #[test]
    fn with_no_equipped_context_each_slots_own_top_pick_still_shows() {
        let classes = vec!["Warrior".to_string()];
        let recs = recommend(&classes, None, None, 200, None, None, None, None, None);
        for slot in ["FINGER1", "FINGER2"] {
            let r = recs.iter().find(|r| r.slot == slot).unwrap();
            assert!(!r.items.is_empty(), "{slot} should still have candidates");
        }
    }

    /// why: owned count on a DTO reflects the real dump, not just presence
    #[test]
    fn owned_count_is_summed_across_the_whole_dump_not_just_equipped() {
        let mut owned = HashMap::new();
        owned.insert("Brass Ring".to_string(), 3u32);
        let classes = vec!["Warrior".to_string()];
        let recs = recommend(
            &classes,
            None,
            None,
            200,
            None,
            None,
            None,
            Some(&owned),
            None,
        );
        let item = recs
            .iter()
            .find(|r| r.slot == "FINGER1")
            .and_then(|r| r.items.iter().find(|it| it.item.name == "Brass Ring"))
            .expect("Brass Ring fits FINGER1");
        assert_eq!(item.item.owned, 3);

        let other = recs
            .iter()
            .find(|r| r.slot == "FINGER1")
            .and_then(|r| r.items.iter().find(|it| it.item.owned == 0));
        assert!(
            other.is_some(),
            "an unowned item should report owned=0, not inherit 3"
        );
    }
}

#[cfg(test)]
mod owned_tier_tests {
    use super::*;

    /// why: an item already upgraded scores and displays at its real tier,
    /// not as a fresh +0 -- real catalog item (Brass Ring, CHA +1, ALL
    /// classes), custom weights so the test controls exactly what's scored
    /// instead of depending on a real class's own CHA weight.
    #[test]
    fn a_real_owned_item_is_scored_and_shown_at_its_owned_tier() {
        let mut weights = HashMap::new();
        weights.insert("CHA".to_string(), 1.0);
        let mut owned_tier = HashMap::new();
        owned_tier.insert("Brass Ring".to_string(), 5u8);
        let classes = vec!["Warrior".to_string()];

        let recs = recommend(
            &classes,
            None,
            None,
            200,
            Some(weights.clone()),
            None,
            None,
            None,
            Some(&owned_tier),
        );
        let owned_item = recs
            .iter()
            .find(|r| r.slot == "FINGER1")
            .and_then(|r| r.items.iter().find(|it| it.item.name == "Brass Ring"))
            .expect("Brass Ring fits FINGER1");
        assert_eq!(
            owned_item.item.tier, 5,
            "DTO should carry the real owned tier"
        );
        assert_eq!(
            owned_item.item.stats.get("CHA"),
            Some(&6.0),
            "CHA +1 scaled to tier 5 is +6 (scale_stat's own formula)"
        );
        assert_eq!(owned_item.score, 6.0, "scored at the owned tier, not base");

        // Same item, same weights, no ownership context -- must score at
        // base (tier 0), proving the difference above came from tier, not
        // a fluke of Brass Ring's own stats.
        let baseline = recommend(
            &classes,
            None,
            None,
            200,
            Some(weights),
            None,
            None,
            None,
            None,
        );
        let base_item = baseline
            .iter()
            .find(|r| r.slot == "FINGER1")
            .and_then(|r| r.items.iter().find(|it| it.item.name == "Brass Ring"))
            .expect("Brass Ring fits FINGER1");
        assert_eq!(base_item.item.tier, 0);
        assert_eq!(
            base_item.score, 1.0,
            "unscaled CHA +1 with no ownership context"
        );
    }
}

#[cfg(test)]
mod scale_weight_tests {
    use super::*;

    #[test]
    fn shrinks_by_the_same_cumulative_fraction_stats_grow_by() {
        // 2.0 wt at tier 5: 2.0 * (1 - 0.5) = 1.0.
        assert_eq!(scale_weight(2.0, 5), 1.0);
    }

    #[test]
    fn never_reaches_zero() {
        // 0.5 wt at tier 10 would be 0.0 unfloored -- the game guarantees
        // a minimum of 0.1.
        assert_eq!(scale_weight(0.5, 10), 0.1);
    }

    #[test]
    fn untiered_is_left_alone() {
        assert_eq!(scale_weight(3.4, 0), 3.4);
    }
}

#[cfg(test)]
mod exalt_slot_tests {
    use super::*;

    /// why: real proc-only item req tier 4 -- locked below it, shows at or above
    #[test]
    fn a_procs_own_socket_stays_locked_below_its_required_tier() {
        let item = itemdata::items()
            .iter()
            .find(|it| it.name == "A Dark Reaver")
            .expect("fixture item present in the real catalog");
        let below = exalt_slots(item, 3);
        let proc = below.iter().find(|s| s.key == "proc").unwrap();
        assert!(!proc.unlocked);
        assert!(proc.effect.is_none());

        let at = exalt_slots(item, 4);
        let proc = at.iter().find(|s| s.key == "proc").unwrap();
        assert!(proc.unlocked);
        assert_eq!(
            proc.effect.as_ref().map(|e| e.name.as_str()),
            Some("Steal Strength")
        );
    }

    /// why: ornamentation is a cosmetic token slot, always None, unlocked from tier 0
    #[test]
    fn ornamentation_never_has_a_native_effect() {
        let item = itemdata::items()
            .iter()
            .find(|it| it.name == "A Dark Reaver")
            .unwrap();
        let slots = exalt_slots(item, 10);
        let orn = slots.iter().find(|s| s.key == "ornament").unwrap();
        assert!(orn.unlocked);
        assert!(orn.effect.is_none());
    }
}

#[cfg(test)]
mod item_at_tier_tests {
    use super::*;

    #[test]
    fn re_derives_a_real_item_at_a_chosen_tier() {
        let item = itemdata::items()
            .iter()
            .find(|it| it.name == "Brass Ring")
            .expect("fixture item present in the real catalog");
        let dto = item_at_tier(&item.id, 5).expect("real id resolves");
        assert_eq!(dto.tier, 5);
        assert_eq!(dto.stats.get("CHA"), Some(&6.0));
        assert_eq!(dto.owned, 0, "no dump context -- always reports unowned");
    }

    #[test]
    fn a_tier_above_10_is_clamped() {
        let item = itemdata::items()
            .iter()
            .find(|it| it.name == "Brass Ring")
            .unwrap();
        let dto = item_at_tier(&item.id, 200).unwrap();
        assert_eq!(dto.tier, 10);
    }

    #[test]
    fn an_unknown_id_is_none() {
        assert!(item_at_tier("not-a-real-item-id", 5).is_none());
    }
}

#[cfg(test)]
mod era_filter_tests {
    use super::*;

    /// why: a real later-era item hidden by default, shown once "All" is selected
    #[test]
    fn all_bypasses_the_default_current_era_ceiling() {
        let velious_item = itemdata::items()
            .iter()
            .find(|it| era_index(it).is_some_and(|ix| ix > era_ix(CURRENT_ERA).unwrap()))
            .expect("catalog has at least one item past the current era");

        let default = list_items(&[], None, None, None, None);
        assert!(
            !default.iter().any(|dto| dto.id == velious_item.id),
            "a later-era item must not show up under the default era ceiling"
        );

        let all = list_items(&[], None, Some("All"), None, None);
        assert!(
            all.iter().any(|dto| dto.id == velious_item.id),
            "the same item must show up once era filtering is turned off"
        );
    }

    /// why: a specific earlier era still narrows -- not just an All/current binary
    #[test]
    fn a_specific_era_name_filters_to_its_own_ceiling() {
        let classic_ix = era_ix("Classic Era").unwrap();
        let past_classic = itemdata::items()
            .iter()
            .find(|it| era_index(it).is_some_and(|ix| ix > classic_ix))
            .expect("catalog has at least one item past Classic Era");

        let classic_only = list_items(&[], None, Some("Classic Era"), None, None);
        assert!(
            !classic_only.iter().any(|dto| dto.id == past_classic.id),
            "an item past Classic Era must not pass a Classic Era ceiling"
        );
    }
}

#[cfg(test)]
mod proc_evidence_tests {
    use super::*;

    fn parsed_with(item: &str, tier: u8) -> crate::inventory::ParsedInventory {
        let mut equipped = HashMap::new();
        equipped.insert(
            "NECK".to_string(),
            crate::inventory::InventoryItem {
                name: item.to_string(),
                tier,
            },
        );
        crate::inventory::ParsedInventory {
            equipped,
            ..Default::default()
        }
    }

    #[test]
    fn an_item_that_has_fired_its_proc_carries_that_evidence() {
        let mut procs = ExaltationProcs::default();
        procs.observe(12_345, "A Bone Necklace".to_string());
        procs.observe(99_999, "A Bone Necklace".to_string());

        let parsed = parsed_with("A Bone Necklace", 5);
        let dto = resolve_inventory(&parsed, Some(&procs));
        let item = dto.resolved.get("NECK").unwrap();

        let evidence = item.proc_evidence.as_ref().expect("this item fired twice");
        assert_eq!(evidence.fires, 2);
        assert_eq!(evidence.first_seen_ms, 12_345);
    }

    #[test]
    fn an_item_with_no_fires_carries_no_evidence() {
        let procs = ExaltationProcs::default();
        let parsed = parsed_with("A Bone Necklace", 5);
        let dto = resolve_inventory(&parsed, Some(&procs));
        assert!(dto.resolved.get("NECK").unwrap().proc_evidence.is_none());
    }

    #[test]
    fn no_proc_context_at_all_still_resolves_cleanly() {
        let parsed = parsed_with("A Bone Necklace", 5);
        let dto = resolve_inventory(&parsed, None);
        assert!(dto.resolved.get("NECK").unwrap().proc_evidence.is_none());
    }
}

#[cfg(test)]
mod exalt_candidate_tests {
    use super::*;

    /// why: real ALL-class FINGERS target and a legal focus source with zero narrowing
    #[test]
    fn a_same_slot_same_class_source_is_a_legal_candidate() {
        let candidates = exalt_candidates("Brass_Ring", "focus", &HashMap::new(), &[], None);
        assert!(
            candidates.iter().any(|c| c.name == "Adamantite Band"),
            "an ALL-class, FINGERS-slot focus source must be offered for a FINGERS ring"
        );
    }

    /// why: a SECONDARY-only source onto a FINGERS ring would leave nowhere to wear it
    #[test]
    fn a_source_with_no_overlapping_slot_is_illegal() {
        let candidates = exalt_candidates("Brass_Ring", "focus", &HashMap::new(), &[], None);
        assert!(
            !candidates.iter().any(|c| c.name == "A Shimmering Orb"),
            "a SECONDARY-only source must not be offered for a FINGERS-only ring"
        );
    }

    /// why: a candidate legal in the abstract but for an unplayed class must not be offered
    #[test]
    fn a_source_usable_by_a_different_class_than_the_player_is_excluded() {
        // why: "All" era isolates the class filter from the separately tested era one
        let candidates = exalt_candidates(
            "Brass_Ring",
            "click",
            &HashMap::new(),
            &["Warrior".to_string()],
            Some("All"),
        );
        assert!(!candidates
            .iter()
            .any(|c| c.name == "Band of Screaming Winds"));
        // why: WAR is one of its classes, must still show up
        assert!(candidates.iter().any(|c| c.name == "Berserkers Ring"));
    }

    #[test]
    fn an_item_is_never_offered_as_its_own_exaltation_source() {
        let candidates = exalt_candidates("Adamantite_Band", "focus", &HashMap::new(), &[], None);
        assert!(!candidates.iter().any(|c| c.name == "Adamantite Band"));
    }

    #[test]
    fn an_unknown_socket_key_returns_nothing() {
        assert!(exalt_candidates(
            "Brass_Ring",
            "not-a-real-socket",
            &HashMap::new(),
            &[],
            None
        )
        .is_empty());
    }

    #[test]
    fn ornament_has_no_candidates_at_all() {
        // why: cosmetic token slot, no effect_key, nothing to source
        assert!(exalt_candidates("Brass_Ring", "ornament", &HashMap::new(), &[], None).is_empty());
    }

    /// why: end to end -- socketing a source into a no-native-effect target must show the source
    #[test]
    fn item_with_exalts_shows_the_socketed_sources_own_effect() {
        let mut assignments = HashMap::new();
        assignments.insert("focus".to_string(), "Adamantite_Band".to_string());
        let dto = item_with_exalts("Brass_Ring", 5, &assignments).expect("real catalog id");
        let focus = dto.exalts.iter().find(|e| e.key == "focus").unwrap();
        assert!(
            focus.unlocked,
            "tier 5 is well past focus's own +1 requirement"
        );
        assert_eq!(
            focus.effect.as_ref().map(|e| e.name.as_str()),
            Some("Summoning Haste I"),
            "should carry Adamantite Band's own effect, not Brass Ring's (it has none)"
        );
    }

    /// why: an unassigned socket falls back to native effect -- assigning one must not disturb others
    #[test]
    fn unassigned_sockets_still_show_the_items_own_native_effect() {
        let mut assignments = HashMap::new();
        assignments.insert("focus".to_string(), "Adamantite_Band".to_string());
        // why: real native click effect of its own
        let dto = item_with_exalts("Berserkers_Ring", 5, &assignments).expect("real catalog id");
        let click = dto.exalts.iter().find(|e| e.key == "click").unwrap();
        assert_eq!(
            click.effect.as_ref().map(|e| e.name.as_str()),
            Some("Firefist")
        );
    }
}
