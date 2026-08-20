//! Native gear planner: item browsing, BiS-style recommendation, and a
//! "seen configurations" hookup that pre-selects your classes from the
//! live parse instead of asking you to pick them by hand.
//!
//! The scoring model (`STAT_WEIGHTS`, the tuning constants, `derived_
//! weights`) is a direct port of the standalone planner
//! (`ui/app/planner/`, kept as its own reference copy, not loaded by the
//! app anymore) -- ported faithfully rather than re-derived, since it's
//! the user's own tuned system, not a blank slate. `STAT_WEIGHTS` is
//! stated the same way its source states it: a heuristic opinion about
//! what matters per class, not verified EQL game data (there is no public
//! source for "how much is a point of AC worth to a Paladin" -- this is a
//! judgement call, carried over as-is rather than re-guessed).
//!
//! Deliberately a *subset* of the standalone planner's full feature set:
//! two-hand/dual-wield hand-pairing and exaltation auto-assignment aren't
//! ported. Each is a real, separate chunk of logic in its own right;
//! porting them half-correct under time pressure would be worse than
//! leaving them out and saying so, which is what this module doc does.
//! INT/WIS *diverge* from the standalone planner's flat per-class number,
//! though -- see `derived_weights`'s doc for the mana-pool mechanic (top 2
//! of 3 classes' own mana values, summed) that flat weight couldn't see,
//! confirmed against the user's own real character rather than assumed.
//!
//! LORE duplicates *are* handled (`recommend`'s `claimed_lore`), but as a
//! single greedy pass over `SLOTS` in a fixed order, not the standalone
//! planner's fuller coverage-optimizing assignment -- good enough to never
//! suggest an unwearable loadout, not guaranteed to be the globally best
//! way to spread scarce LORE items across slots that could each use one.

use crate::ingest::ExaltationProcs;
use crate::itemdata::{self, Item};
use eqlp_source::Millis;
use serde::Serialize;
use std::collections::HashMap;

// ---------------------------------------------------------------- classes/races

/// Class code -> full name, the same 15-class roster
/// `ui/app/planner/index.html`'s own `CLASSES` map uses. The *other*
/// direction (full name -> code) is what `recommend`/`list_items`/
/// `weights_for` need, to translate the full names their `classes`
/// parameter takes (the same shape `default_classes` returns, and the
/// same shape the frontend's class chips are keyed by) into this module's
/// codes.
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
/// `pub(crate)`, not private: `crate::character`'s own estimate needs the
/// same name -> code translation this module already owns, rather than a
/// second copy that could drift from `CLASS_NAMES`.
pub(crate) fn name_to_code(name: &str) -> Option<&'static str> {
    CLASS_NAMES
        .iter()
        .find(|(_, n)| *n == name)
        .map(|(c, _)| *c)
}

/// The same 15-race roster `recommend`'s own local table used to carry
/// inline -- pulled out to a module-level const so `crate::character` can
/// translate a race name the same way this module does, instead of a
/// second hand-copied list that could quietly drift out of sync with this
/// one (exactly the kind of gap `CLASS_NAMES` vs. a duplicate class list
/// would also risk, if `crate::character` had needed its own).
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

/// `["Wizard", "Enchanter", "Magician"]` (full names) -> `["WIZ", "ENC",
/// "MAG"]` (this module's codes) -- names that don't match any known class
/// are dropped rather than guessed at.
fn names_to_codes(names: &[String]) -> Vec<String> {
    names
        .iter()
        .filter_map(|n| name_to_code(n))
        .map(String::from)
        .collect()
}

/// Every class in `classes` this game actually has above level 10 (from
/// `eqlp_session::classdetect`), as full class names -- the same shape
/// `recommend`/`list_items` take in `classes` (they do their own
/// name-to-code translation) and the same shape the frontend's class chips
/// are keyed by. Returning codes here instead once meant the chips could
/// never match a default against their own labels: nothing would render as
/// selected, yet the 3-class cap still saw a full `classes` array and
/// disabled every chip -- "stuck at 3 selected" with none actually picked.
/// Empty if `name` hasn't confirmed a configuration yet -- the planner
/// already handles "no classes selected" (shows everything unfiltered), so
/// this deliberately doesn't fall back to a guess.
pub fn default_classes(ing: &crate::ingest::Ingest, name: &str) -> Vec<String> {
    let Some(sym) = ing.store.names.get(name) else {
        return Vec::new();
    };
    let (resolved, _) = ing.classes.visits_by_resolved_configuration(sym.0);
    let Some((dominant, _)) = resolved.into_iter().next() else {
        return Vec::new();
    };
    dominant
}

/// `item.classes` is either a plain code list, `["ALL"]` (every class), or
/// `["ALL_EXCEPT", <excluded codes...>]` (every class but these) --
/// confirmed against real entries in `packs/items.json`, not assumed. The
/// only place in this module that sentinel gets interpreted.
fn usable_by(item: &Item, active: &[String]) -> bool {
    if active.is_empty() {
        return true; // no filter selected -- show everything
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

/// `item.classes`' own `ALL`/`ALL_EXCEPT` sentinels, expanded to the
/// concrete code list they actually mean -- what exaltation narrowing
/// (`intersect_classes`, below) needs, since intersecting a literal
/// `["ALL"]` against a real code list with plain list intersection would
/// wrongly produce an empty result instead of "everything `b` allows".
/// Mirrors the standalone planner's own `cx()` doing this inline; split
/// out here since this module intersects in more than the one place that
/// function did.
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

/// Real list intersection, both sides already expanded past `ALL`/
/// `ALL_EXCEPT` (see `expand_classes`) -- an exaltation's class list
/// narrows the target's own to whatever both sides allow. An empty
/// result means the swap would leave nothing able to wear the item at
/// all, which `exalt_candidates` treats as illegal, not just narrowed.
fn intersect_str(a: &[String], b: &[String]) -> Vec<String> {
    a.iter().filter(|x| b.contains(x)).cloned().collect()
}

/// `item`'s own class/slot lists, narrowed by every exaltation in
/// `assignments` *except* `exclude_key` (the socket a candidate is being
/// chosen for right now -- its own not-yet-committed pick obviously isn't
/// part of "what's already narrowing this item"). Mirrors the standalone
/// planner's `effective()`, minus the stat-scaling half of that function
/// (`ItemDto`'s own tier scaling already covers that elsewhere).
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

/// A summoned item's effect can't be extracted -- you can't level a
/// second copy of something a spell conjures, so there's no exaltation
/// to pull out of it. Mirrors the standalone planner's own `isSummoned`
/// (`TEMPORARY` tag, or a name starting with "Summoned:" for the wiki
/// pages missing that tag -- confirmed both signals are needed, neither
/// alone catches every real case).
fn is_summoned(item: &Item) -> bool {
    item.tags.iter().any(|t| t == "TEMPORARY") || item.name.to_lowercase().starts_with("summoned")
}

fn race_ok(item: &Item, race: Option<&str>) -> bool {
    let Some(race) = race else { return true };
    item.races.is_empty() || item.races.iter().any(|r| r == "ALL" || r == race)
}

/// The wiki's own "doesn't actually exist here" marker. eqlwiki is a fork
/// of a P99 dataset, and a page like Orb of Draconic Energy can carry full
/// stats, a real era-looking layout, and *no* Era category at all, yet
/// never have been implemented on this server -- `Category:Non-P99
/// Content` is how the wiki itself flags that, separately from era. Era
/// filtering can't substitute for this: `in_era` treats an item with no
/// resolved era as always-in-era (see its own doc), so an unimplemented
/// item with no Era category sailed straight through as "current" instead
/// of being caught by any era check.
fn on_server(item: &Item) -> bool {
    !item.categories.iter().any(|c| c == "Non-P99 Content")
}

/// Single toggle for the whole app -- flip to `true` once Imbue spells go
/// live, no per-item changes needed (`Item::requires_imbue` already tracks
/// which items need it, recomputed fresh on every re-scrape).
const IMBUE_SPELLS_LIVE: bool = false;

fn imbue_ok(item: &Item) -> bool {
    IMBUE_SPELLS_LIVE || !item.requires_imbue
}

/// LORE and LORE_EQUIPPED both mean the same restriction (you can't have
/// two copies of *this* item equipped at once -- LORE_EQUIPPED is EQ's own
/// finer-grained variant, worn-slot-specific, but the "no second copy"
/// rule reads identically for this module's purposes). Enforced per item
/// *name*, not "one LORE item total" -- see `recommend`'s `claimed_lore`
/// for where that distinction matters.
fn is_lore(item: &Item) -> bool {
    item.tags
        .iter()
        .any(|t| t == "LORE" || t == "LORE_EQUIPPED")
}

// ---------------------------------------------------------------- era

/// Chronological by the live-EQ dates the wiki's own era categories
/// correspond to -- direct port of the standalone planner's `ERA_ORDER`.
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

/// Where EQ Legends actually is right now -- the standalone planner's own
/// `CURRENT_ERA`, carried over as-is (it's a fact about the live server at
/// the time that constant was set, not derived from anything scraped).
pub const CURRENT_ERA: &str = "Sky Era";

fn era_ix(era: &str) -> Option<usize> {
    ERA_ORDER.iter().position(|e| *e == era)
}

/// The earliest era this item is known to exist in, or `None` if that's
/// genuinely unresolvable from the scrape. Prefers `available_from`
/// (the scraper's own best answer) over the raw `eras` list's minimum
/// (multiple era categories on one page, earliest wins) over the single
/// `era` field.
///
/// Deliberately simpler than the standalone planner's full resolution
/// chain: that version also carries a small hardcoded override list for
/// items EQL made available earlier than their wiki era, and a
/// zone-name-to-era voting fallback for the ~30% of pages with no era
/// category at all (see its own doc for why: an item's zone is a much
/// weaker signal than its own category, and a zone that doesn't reach an
/// 80% majority is left unresolved rather than guessed at). Neither is
/// ported -- an item this can't resolve is left `None` and always shown,
/// matching that same "unresolved, not hidden" stance, just without the
/// zone-voting step that occasionally resolves what this leaves open.
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

/// Whether `item` exists at or before `max_era` -- `None` defaults to
/// `CURRENT_ERA`, matching `preferences::Preferences::era`'s own default
/// (a fresh install with no era preference saved yet browses the live
/// server's own current era, not the unfiltered whole catalog). `Some(
/// "All")` is the Settings module's explicit "every era" choice -- not a
/// name `ERA_ORDER` carries, checked before the lookup below rather than
/// left to fall out of `era_ix` returning `None` for an unrecognized
/// string, so this bypass reads as an intentional case, not an
/// unrecognized-input fallback that happens to also do the right thing.
/// An item whose own era is unresolved (see `era_index`'s doc) still
/// always passes either way, since there's nothing there to compare
/// against and hiding it would be a guess in the other direction.
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

/// Maps a gear-planner slot key (`EAR1`, `WRIST2`, `ANY1`, ...) to the
/// `item.slots` token that fills it -- `EAR1`/`EAR2` both accept `EAR`,
/// same physical-slot-pairing the dump parser in `inventory.rs` already
/// uses, confirmed against real `packs/items.json` slot tokens (`EAR`,
/// `WRIST`, `FINGERS` -- plural, unlike the planner's singular `FINGER1`/
/// `FINGER2` keys). `ANY1`/`ANY2` are flex slots that accept anything with
/// at least one real equip slot (not a purely cosmetic/no-slot item).
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

/// One class code's own opinionated stat priorities -- see this module's
/// doc for why this is carried over as-is from the standalone planner
/// rather than re-derived: a heuristic judgement call, not verified game
/// data, same caveat its source states.
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

/// Which stat a class's own mana pool is computed from, if any -- read
/// straight off its `stat_weights` row (whichever of INT/WIS appears there
/// with a positive weight) rather than a second hand-maintained table, so
/// the two can't quietly drift apart. `None` for a class with no mana pool
/// at all (WAR/MNK/ROG/BER: no MANA entry in `stat_weights` either).
/// `pub(crate)`, not private: `crate::character`'s own naked-stat mana
/// estimate uses this exact same per-class casting-stat lookup, rather
/// than a second hand-maintained copy -- see this function's own doc for
/// why it's derived from `stat_weights` instead of a dedicated table in
/// the first place.
pub(crate) fn casting_stat(code: &str) -> Option<&'static str> {
    if !uses_mana(code) {
        return None;
    }
    stat_weights(code)
        .iter()
        .find_map(|&(k, v)| (v > 0.0 && (k == "INT" || k == "WIS")).then_some(k))
}

/// The real in-game mana formula lives in `crate::manadata` now, not here
/// -- a spreadsheet the user found (reconstructed from this server's own
/// `EQEmu`-derived client formulas) turned out to have it exactly, which
/// superseded three straight rounds of this module reverse-engineering an
/// approximation from play data alone (5, then 2.5, then a two-part
/// level-base-plus-stat-bonus fit -- see git history / `manadata`'s own
/// module doc for what each of those got right and wrong). Verified
/// against nine real measurements: four of five absolute naked readings
/// matched exactly, the fifth within 0.3%, every delta consistent.
///
/// What's left here is `mana_marginal_rate`, a *scoring* helper -- how
/// much one more point of INT/WIS is worth for ranking gear, not a pool
/// reconstruction, so it doesn't need (and can't cheaply have) a real
/// character's actual current stat total the way `manadata::class_mana_
/// pool` does.
fn mana_marginal_rate(active: &[String], level: u8) -> f64 {
    // A fixed reference stat, not any one character's real total -- this
    // is a relative weight for ranking items against each other, not an
    // attempt to match a specific pool number, so where exactly on the
    // curve it's measured matters far less than that every class is
    // measured at the same point.
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

/// The class-derived starting weight vector for `active` (up to 3 class
/// codes), `level` from `Ingest::levels` (`None` if no `level.up` line has
/// been seen yet this session -- see that method's doc). AC/HP/MANA are
/// the mean across the active classes (defensive stats the whole trio
/// shares); most other stats are the max (only the class that actually
/// wants a given stat should drive its number) -- INT/WIS are the one
/// exception, and only once `level` is known.
///
/// A flat per-class INT/WIS number (the max-based rule every other stat
/// uses) can't represent this game's actual mana mechanic: your usable
/// mana pool is the sum of the *top two* of your three classes' own mana
/// values, each computed from the real formula in `crate::manadata` (see
/// that module's own doc). Two or three of your classes drawing mana
/// from the *same* stat means growing that stat helps two pool-slots at
/// once -- a real, roughly double return a flat weight has no way to see,
/// and exactly the case that prompted this (three INT-casters at once).
///
/// So once `level` is known, INT/WIS stop being flat per-class priorities
/// and become *purely* derived from the mana math: `mana_per_point` (the
/// same number `w["MANA"]` below gets, i.e. how much one point of actual
/// mana pool is worth to this loadout) times how much pool a point of that
/// stat actually buys. That "how much pool" multiplier is capped at 2 (only
/// two slots exist) and handles the classes-share-a-stat cases directly:
///
/// - Only one of your classes uses a given stat (or two, on different
///   stats -- e.g. one INT class + one WIS class + one non-caster): no
///   contest, that stat fills exactly one slot, multiplier 1.
/// - Two or three classes share a stat and it's the only casting stat in
///   play (or shared by strictly more classes than the other one): that
///   stat is treated as filling *both* slots, multiplier 2, and the other
///   (minority) stat gets 0 -- it can plan to end up in your top two, but
///   this app has no live INT/WIS totals to know if it currently does, and
///   optimizing toward the stat more of your build already leans on is the
///   more useful default for a planning tool than guessing.
/// - Equal counts on both stats (the only way that happens with 3 classes
///   is one-and-one, with a non-caster third) aren't a contest at all --
///   each of your two real casters just gets its own slot outright, so
///   both stats get multiplier 1, not 2 and 0.
///
/// Because both INT/WIS and MANA all resolve through the same
/// `mana_per_point` number, a point of raw +MANA and the mana-equivalent
/// value of a point of INT/WIS are directly comparable instead of two
/// independently-tuned numbers that could double-count the same pool.
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

/// A single number for how good `item` is against `weights` -- dot
/// product of the item's stats against the weight vector, plus a weapon's
/// damage/delay ratio and a flat bonus per scored effect slot present
/// (proc excluded, same reasoning `derivedWeights`'s source states: proc
/// is graftable via exaltation onto whatever wins on stats, so scoring it
/// directly would let a mediocre item win purely on an effect that isn't
/// actually tied to it).
///
/// `tier` scales stats/damage the same way an equipped dump's own real
/// tier does (`scale_stat`) before scoring -- `0` (the catalog's own base
/// stats) for a plain browse/recommend, the real owned tier for an item
/// the player already has upgraded, so a copy sitting at +8 scores as the
/// +8 item it actually is, not as if it were freshly dropped at +0.
fn score_item(item: &Item, weights: &HashMap<String, f64>, tier: u8) -> f64 {
    let mut s = 0.0;
    for (stat, val) in &item.stats {
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

/// A tier-0 stat/damage value scaled to `tier` (0-10, the game's own "+N"
/// item-upgrade system) -- ported from the standalone planner's own
/// `scaleStat` (`ui/app/planner/index.html`), which the user re-confirmed
/// the shape of: +10% of base per tier, minimum +1 per tier (so a stat too
/// small for 10% to round to anything still climbs), whichever of the two
/// is larger -- at +10 that's `max(2x base, base + 10)`, "essentially
/// doubling, or adding 10, whichever is more". Equivalent to that source's
/// own `atTier` (its "mid-tier creep" half, `cumBonus`'s fractional-XP
/// interpolation, doesn't apply here: an equipped item's dump only ever
/// reports its settled integer tier, never the exact XP within it, so
/// there's nothing to interpolate between -- see `crate::inventory::
/// InventoryItem::tier`'s own doc).
///
/// Rounds to the nearest whole number, not down -- corrected against a
/// real item the user checked in-game: Tobrin's Mystical Eyepatch, base
/// INT 15, is 17 at +1, not 16. `15 * 1.1 = 16.5`; the standalone
/// planner's own source used `Math.floor` here (16), which this had
/// carried over unquestioned -- wrong the moment the 10% step lands
/// exactly on a half-point, which `Umbral Platemail Breastplate`'s own
/// worked example (AC 30, an exact multiple of 10) could never have
/// caught. `f64::round` breaks ties away from zero, matching this.
///
/// The negative branch mirrors the source's own explicit caveat: eqlwiki
/// publishes no negative-stat worked example, so applying the same +10%-
/// per-tier growth to a penalty (deepening it as the item upgrades) is
/// what the formula says taken literally, not independently confirmed.
/// Carried over as-is rather than guessed at differently.
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

/// The other half of the standalone planner's `scaledItem` (`ui/app-
/// legacy/planner/index.html`): weight *shrinks* as an item tempers up,
/// same cumulative-bonus fraction `scale_stat` grows stats by (at an
/// item's own settled tier, with no partial xp into the next one, that
/// fraction is just `tier / 10`), floored at 0.1 so an item never reaches
/// zero weight. Rounded to one decimal place, matching the source's own
/// `Math.round(... * 10) / 10`.
fn scale_weight(base: f64, tier: u8) -> f64 {
    if tier == 0 {
        return base;
    }
    let shrunk = base * (1.0 - tier as f64 / 10.0);
    ((shrunk * 10.0).round() / 10.0).max(0.1)
}

/// One item's own native effect, in the shape the doll/preview panel
/// actually needs -- `Item::effects`' raw `serde_json::Value` (whatever
/// shape `scrape.py` happened to capture per key: always `name`,
/// sometimes `detail`/`level` alongside it) narrowed down to what's ever
/// displayed. `None` if the key is present but somehow missing even a
/// name -- treated as absent rather than a panic, since this is scraped
/// data, not something this app controls the shape of.
#[derive(Debug, Clone, Serialize)]
pub struct ItemEffectDto {
    pub name: String,
    pub detail: Option<String>,
}

fn item_effect(item: &Item, key: &str) -> Option<ItemEffectDto> {
    let v = item.effects.get(key)?;
    Some(ItemEffectDto {
        name: v.get("name")?.as_str()?.to_string(),
        detail: v
            .get("detail")
            .and_then(|d| d.as_str())
            .map(String::from),
    })
}

/// The exaltation socket family, in unlock order -- `eqlwiki.com/
/// Exaltations`' own thresholds, ported from the standalone planner's
/// `EXALTS` (`ui/app-legacy/planner/index.html`). `effect_key` is which
/// of `Item::effects`' keys that socket type corresponds to; the
/// Ornamentation slot has none -- it takes a cosmetic token, not an
/// effect, so it never has a "native" occupant to report.
const EXALT_SLOTS: &[(&str, &str, u8, Option<&str>)] = &[
    ("ornament", "Ornamentation", 0, None),
    ("focus", "Focus", 1, Some("focus")),
    ("click", "Click", 2, Some("click")),
    ("worn", "Worn", 3, Some("worn")),
    ("proc", "Proc", 4, Some("proc")),
];

/// One socket's state for `item` at `tier`: whether upgrading has opened
/// it yet, and -- if it corresponds to an effect type the item actually
/// carries -- that effect. This only ever reports the item's *own*
/// effect occupying its *own* socket; grafting a different item's effect
/// into an open-but-empty socket ("exaltation" proper, the standalone
/// planner's `autoExaltAll`) is real EQL gear-planning behavior this
/// module deliberately doesn't attempt yet -- see this module's own doc
/// comment. An open socket with no native effect just reports `unlocked:
/// true, effect: None`: room for one, nothing assigned by this app.
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
    /// First drop source, formatted the same "zone — mob" shape the
    /// standalone planner's slot badge used, for the same reason: a mob
    /// drop takes priority when there is one, otherwise whatever real
    /// source (quest/vendor/crafted) the item actually has.
    pub source: Option<String>,
    /// Every zone `item.drops` names, deduplicated, in scrape order.
    /// `source` above only ever surfaces the first one -- these two plus
    /// `mobs` are what the alt list's two source lines render, since a
    /// drop can span more than one zone and this app shouldn't hide that.
    pub zones: Vec<String>,
    /// Every mob `item.drops` names across all its zones, deduplicated,
    /// in scrape order.
    pub mobs: Vec<String>,
    pub url: Option<String>,
    pub wt: Option<f64>,
    pub size: Option<String>,
    /// The "+N" this specific instance is at -- always `0` for a browsed/
    /// recommended item (wiki-scraped base stats, before any upgrading),
    /// the real dump-reported tier for an equipped one from `resolve_
    /// inventory`. `stats`/`dmg` above are already scaled to this tier
    /// (see `scale_stat`) -- `tier` is carried alongside them for display
    /// (an equipped item's own "+N" badge), not as a caller's cue to
    /// scale anything itself.
    pub tier: u8,
    /// Total copies owned anywhere (bags/bank/equipped), from the last
    /// loaded `/outputfile inventory` dump -- `0` with no dump loaded, or
    /// genuinely `0` owned. Never a same-slot-only count: a ring owned
    /// once shows `1` even while equipped in the other ring slot, so the
    /// UI can tell "own one, need a second" from "own none at all".
    pub owned: u32,
    /// This item's own native effects, keyed `"focus"/"click"/"worn"/
    /// "proc"` -- tier-independent (an item either has a given effect or
    /// it doesn't; upgrading only unlocks the *socket* that exposes it,
    /// see `exalts` below). Straight from `Item::effects`, just narrowed
    /// to the shape the preview panel actually renders.
    pub effects: HashMap<String, ItemEffectDto>,
    /// The 5 exaltation sockets (`EXALT_SLOTS`), evaluated at this DTO's
    /// own `tier` -- which are open yet, and which hold this item's own
    /// effect. See `exalt_slots`' doc for what this does and doesn't
    /// model.
    pub exalts: Vec<ExaltSlotDto>,
    /// Real-session evidence that this specific equipped item's Proc
    /// exaltation socket has actually fired -- `None` for every browsed/
    /// recommended item (this is only ever filled in by `resolve_
    /// inventory`, the one place a "this instance, right now" item
    /// exists at all) and for an equipped item whose proc simply hasn't
    /// fired yet this session. See `ProcEvidenceDto`'s own doc for
    /// exactly what this can and can't say.
    pub proc_evidence: Option<ProcEvidenceDto>,
}

/// What a real "Your `<item>` (Exaltation) ..." combat-proc line can
/// prove -- see `crate::ingest::ExaltationProcs`' own doc for the full
/// story of why this is the *only* per-item exaltation fact this app can
/// ever confirm: never which effect resulted, only that the socket is
/// genuinely live and how many times it's fired.
#[derive(Debug, Clone, Serialize)]
pub struct ProcEvidenceDto {
    pub fires: u32,
    pub first_seen_ms: Millis,
}

/// First-occurrence dedup, order preserved -- small lists (a handful of
/// zones/mobs per item at most), so a HashSet-backed filter is plenty.
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

/// `tier` is `0` for every caller except `resolve_inventory` -- see
/// `ItemDto::tier`'s own doc. At `0`, `scale_stat` is a no-op, so this is
/// exactly the old always-base-stats behavior for browsing/recommending.
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

/// A single catalog item, re-derived at a caller-chosen `tier` (clamped
/// to the game's own 0-10 range) -- the "level this up and show me" what-
/// if the doll's tier picker drives, independent of whatever tier the
/// item actually sits at in a loaded dump. `None` if `id` doesn't match
/// anything in the catalog (stale/bad id from the frontend). Always
/// reports `owned: 0`: this call has no dump context of its own, and the
/// frontend already has the real owned count from wherever it got this
/// item in the first place -- it's the tier-dependent fields (`stats`,
/// `dmg`, `wt`, `tier`, `exalts`) this exists to refresh, not ownership.
pub fn item_at_tier(id: &str, tier: u8) -> Option<ItemDto> {
    let item = itemdata::by_id(id)?;
    Some(to_dto(item, tier.min(10), 0))
}

/// `exalt_slots`' own logic, but a socket with a real assignment shows
/// *that source's* effect instead of the item's native one -- what
/// actually happens in-game when you exalt a different item's effect
/// into an open socket. Kept separate from `exalt_slots` itself (not an
/// added parameter there) so every existing caller/test of the plain
/// native-effect-only path stays untouched.
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
                itemdata::by_id(source_id).and_then(|src| effect_key.and_then(|k| item_effect(src, k)))
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

/// `item_at_tier`'s own DTO, with `exalts` re-derived against a set of
/// "what if I socket this other item's effect here" assignments (socket
/// key -> source item id) instead of the item's own native effects. Every
/// other field (`stats`/`dmg`/`wt`/`tier`) is unaffected by exaltation --
/// only the socket contents change, which is exactly what
/// `exalt_slots_with_assignments` recomputes.
pub fn item_with_exalts(id: &str, tier: u8, assignments: &HashMap<String, String>) -> Option<ItemDto> {
    let item = itemdata::by_id(id)?;
    let tier = tier.min(10);
    let mut dto = to_dto(item, tier, 0);
    dto.exalts = exalt_slots_with_assignments(item, tier, assignments);
    Some(dto)
}

/// Legal exaltation sources for `socket_key` on `item_id`, given whatever
/// *other* sockets on that same item are already assigned (`other_
/// assignments` -- excludes `socket_key` itself, since a not-yet-
/// committed pick for the very socket being filled obviously isn't part
/// of "what's already narrowing this item"). Mirrors the standalone
/// planner's own `legalSources`: a candidate must carry the matching
/// effect type, and intersecting its class/slot list with the target's
/// own (already-narrowed) list must leave something real -- an empty
/// intersection means the swap would make the item unwearable outright,
/// so it isn't offered at all, not just flagged.
///
/// `classes` (full names) both filters the candidate by the *player's*
/// active trio (no point suggesting a source only a class you're not
/// playing could equip) and, combined with `expand_classes`, is what
/// makes an `ALL`/`ALL_EXCEPT` source narrow correctly rather than
/// vanishing under plain list intersection. Empty `classes` means no
/// player-class filter at all (matches `usable_by`'s own convention).
///
/// Excludes the item itself (assigning an item's own native effect to
/// its own socket is a no-op already covered by leaving the socket
/// unassigned) and every summoned/out-of-era/off-server source, the same
/// pool `exaltPool()` filters to.
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
    let Some(&(_, _, _, Some(effect_key))) = EXALT_SLOTS.iter().find(|&&(k, ..)| k == socket_key) else {
        return Vec::new(); // unknown socket, or ornament (no effect_key -- nothing to source)
    };
    let (eff_classes, eff_slots) = effective_classes_slots(item, other_assignments, Some(socket_key));
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
    /// Slot key -> the matched item, full DTO -- icon, stats, wiki link,
    /// everything the doll and detail panel need, same shape a
    /// recommendation's own items carry.
    pub resolved: HashMap<String, ItemDto>,
    /// Slot key -> the dump's own printed name, for equipped items that
    /// didn't match anything in this module's catalog by exact name (a
    /// renamed item, a tier variant not in the wiki, a straight scrape
    /// gap). Surfaced rather than silently dropped, so "some of my gear
    /// didn't load" has a concrete answer instead of just a lower count.
    pub unresolved: HashMap<String, String>,
    /// Base item name -> total copies owned, straight from the dump --
    /// `crate::inventory::ParsedInventory::owned`, passed through so the
    /// frontend has ownership counts for items beyond what's equipped
    /// (a spare ring in the bank, say).
    pub owned: HashMap<String, u32>,
    /// Base item name -> highest tier owned, straight from
    /// `ParsedInventory::owned_tier` -- passed through so a call to
    /// `recommend`/`list_items` can score and display an owned candidate
    /// at the tier the player actually has, not a fresh +0.
    pub owned_tier: HashMap<String, u8>,
}

/// Matches a parsed `/outputfile inventory` dump (slot key -> the item
/// name/tier the game itself printed) against this module's own item
/// catalog by exact name (case-insensitive; the dump's own "+N" tier
/// suffix is already stripped by `inventory::parse` before this ever sees
/// the name, kept separately as `inv_item.tier`). That real tier is what
/// makes this different from `list_items`/`recommend`'s own `to_dto`
/// calls: this is the one place stats get scaled to what's *actually*
/// equipped (`scale_stat`) rather than left at wiki-scraped base -- an
/// inventory dump means real gear at a real tier, not a browsing/what-if
/// view.
/// `proc_evidence` is `Ingest::exaltation_procs` -- `None` for any caller
/// with no live session to check against (there currently isn't one;
/// every real call site has an `Ingest`, this stays optional so a test
/// building a bare `ParsedInventory` isn't forced to construct one).
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

/// Every item usable by `classes` (full class names -- `["Wizard",
/// "Enchanter"]`, translated to this module's codes internally), `race`
/// (full race name or `None` for no filter), and at or before `max_era`
/// (an `ERA_ORDER` name, or `None` for no era filter -- see `in_era`'s
/// doc), optionally narrowed to one slot. Empty `classes` means
/// unfiltered, matching `usable_by`'s own "no filter" rule.
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

/// Top `per_slot` candidates for every slot, scored against `classes`'
/// own derived weight vector, or `custom_weights` in place of it if given
/// -- the compact weight row under the doll edits are user overrides, not
/// a request to re-derive from classes (see `derived_weights`'s doc for
/// what that vector means and doesn't mean). LORE duplicates are handled
/// (see `claimed_lore` below): slots are still scored independently, but
/// a LORE item's name, once placed, is removed from every other slot's
/// candidate pool -- you can't actually equip a second copy of it, so a
/// recommendation set that suggested one wouldn't be a wearable loadout.
/// Different LORE items still stack fine; it's only the same name twice
/// that's disallowed.
///
/// `equipped` (slot key -> real item name, from a loaded inventory dump)
/// seeds `claimed_lore` with whatever's *actually* worn, not just this
/// call's own greedy picks -- without it, a LORE ring truly equipped in
/// FINGER1 could still show up as a FINGER2 candidate, since scoring
/// alone has no idea a real copy is already spoken for. `owned` (base
/// item name -> copies owned) feeds `ItemDto::owned` for display; `owned_
/// tier` (base item name -> highest tier owned) feeds both `ItemDto::
/// tier` *and* the score itself (`score_item`'s own `tier` param) -- an
/// item already upgraded to +8 is scored and shown as the +8 item it
/// really is, not as a fresh +0 drop. All three are `None` for a pure
/// browsing/what-if call with no dump loaded.
///
/// why: nets 2H Primary score against best forfeited Secondary (melee only)
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

    // Name -> the one slot key allowed to claim it. Names, not ids -- a
    // LORE item can turn up under more than one entry in packs/items.json
    // (different drop sources scraped as separate rows) and the in-game
    // restriction is on the name regardless. Seeded first from what's
    // *really* equipped (a slot's own real item is never filtered out of
    // its own candidate list), then filled in by each slot's own greedy
    // top pick as the walk goes -- walked in SLOTS order, which puts the
    // two ANY slots last, so a dedicated slot (a finger, a wrist, ...)
    // claims a LORE item before the catch-all slots ever get a turn at it.
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
            // Only the top pick actually claims the name -- an alt further
            // down this same slot's own list isn't being worn, so it
            // shouldn't lock other slots out of it.
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

/// The scoring vector itself, for display -- lets the UI show *why*
/// items are ranked the way they are (see `derived_weights`'s doc for
/// what it is and isn't).
pub fn weights_for(classes: &[String], level: Option<u8>) -> HashMap<String, f64> {
    derived_weights(&names_to_codes(classes), level)
}

#[cfg(test)]
mod scale_stat_tests {
    use super::*;

    /// The wiki's own worked example (see `scale_stat`'s doc, and the
    /// standalone planner's own comment this was ported from): Umbral
    /// Platemail Breastplate, AC 30 -> 33 at tier 1, 36 at tier 2.
    #[test]
    fn matches_the_wiki_worked_example() {
        assert_eq!(scale_stat(30.0, 1), 33.0);
        assert_eq!(scale_stat(30.0, 2), 36.0);
    }

    #[test]
    fn ten_percent_wins_when_it_beats_the_per_tier_floor() {
        // floor(100 * 1.1) = 110, vs. the +1/tier floor of 101 -- 10% wins.
        assert_eq!(scale_stat(100.0, 1), 110.0);
    }

    #[test]
    fn per_tier_floor_wins_when_ten_percent_rounds_too_low() {
        // round(3 * 1.1) = round(3.3) = 3 (10% of 3 rounds down to
        // nothing extra), but the +1/tier floor guarantees at least +1 --
        // 4 wins.
        assert_eq!(scale_stat(3.0, 1), 4.0);
    }

    /// Real item the user checked in-game: Tobrin's Mystical Eyepatch,
    /// base INT 15 (confirmed in packs/items.json), reads 17 at +1, not
    /// 16 -- `15 * 1.1 = 16.5`, and this is what caught `scale_stat`
    /// still using `floor` (16) instead of rounding to the nearest whole
    /// number (17). The wiki's own worked example (AC 30, exact multiples
    /// throughout) could never have caught this -- it never landed on a
    /// half-point.
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
        // No wiki example exists for a negative stat (see this function's
        // own doc) -- this pins the literal reading of the formula, not
        // an independently confirmed in-game number.
        assert_eq!(scale_stat(-20.0, 2), -24.0);
    }

    /// End to end through `resolve_inventory`, not just the isolated
    /// function -- a real catalog item ("A Bone Necklace", AC 2, confirmed
    /// present in packs/items.json) at a real tier, the same path an
    /// actual `/outputfile inventory` dump goes through. AC 2 at +5:
    /// floor(2 * 1.5) = 3 vs. the +1/tier floor of 2 + 5 = 7 -- the floor
    /// wins for a stat this small, same as `per_tier_floor_wins...` above,
    /// just exercised through the real resolve path instead of calling
    /// `scale_stat` directly.
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
            owned: HashMap::new(),
            owned_tier: HashMap::new(),
        };
        let dto = resolve_inventory(&parsed, None);
        let item = dto
            .resolved
            .get("NECK")
            .expect("A Bone Necklace is a real catalog item");
        assert_eq!(item.tier, 5);
        assert_eq!(item.stats.get("AC"), Some(&7.0));
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

        // Same filter chain `recommend`'s own `best_secondary_score`
        // precompute uses -- `in_era`/`race_ok` included even though this
        // call passes `None`/`None` for both, since `None` era means "the
        // current era ceiling", not "unfiltered" (see `in_era`'s own
        // doc) -- an era-locked item the real precompute excludes has to
        // stay excluded here too, or this test would be comparing against
        // a bigger, wrong candidate pool than `recommend` actually used.
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

        // No `per_slot` truncation -- a real, well-scored 2H candidate can
        // legitimately net negative once the best real Secondary
        // (dual-wielding the game's single best-ratio weapon, say) is
        // subtracted off, and would otherwise fall out of a small top-N
        // before this test ever saw it. The netting math is what's under
        // test, not where a 2H happens to rank.
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

    /// The opposite case: a pure caster's RATIO weight is 0 (see
    /// `derived_weights`), so a 2H candidate's score is left exactly as
    /// `score_item` computed it -- no SECONDARY-forfeit adjustment,
    /// because a caster's own Secondary is ordinarily just another
    /// independently-scored stat-stick, not something in a weapon-ratio
    /// contest with Primary the way melee's is.
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

    /// "A Dark Reaver" (confirmed in packs/items.json): a real item whose
    /// only effect is a proc, req tier 4 -- below that tier the proc
    /// socket must report unlocked:false and no effect, even though the
    /// item genuinely has one; at or above it, the effect must show.
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
        assert_eq!(proc.effect.as_ref().map(|e| e.name.as_str()), Some("Steal Strength"));
    }

    /// Ornamentation has no effect key at all -- it's a cosmetic token
    /// slot, not tied to anything `Item::effects` carries -- so it must
    /// always report `effect: None`, unlocked from tier 0 on.
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

    /// A real Velious-era item (a later era than the default `CURRENT_
    /// ERA`/"Sky Era") is hidden with no era override, shown once "All"
    /// is selected -- the Settings module's own era preference, threaded
    /// straight through to `list_items`'s existing `max_era` param (no
    /// new filtering logic needed, this is exercising the thing that was
    /// already there and just never had a UI in front of it).
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

    /// A specific, earlier era name (not "All", not the default) still
    /// narrows the catalog to that ceiling -- picking an era in Settings
    /// isn't just an All/current binary.
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
            owned: HashMap::new(),
            owned_tier: HashMap::new(),
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

    /// Real catalog items (confirmed in packs/items.json): "Brass Ring"
    /// (ALL classes, FINGERS-only, no native effects -- the target) and
    /// "Adamantite Band" (also ALL/FINGERS, a real focus effect -- a
    /// legal source with zero narrowing either way).
    #[test]
    fn a_same_slot_same_class_source_is_a_legal_candidate() {
        let candidates = exalt_candidates("Brass_Ring", "focus", &HashMap::new(), &[], None);
        assert!(
            candidates.iter().any(|c| c.name == "Adamantite Band"),
            "an ALL-class, FINGERS-slot focus source must be offered for a FINGERS ring"
        );
    }

    /// "A Shimmering Orb" is a real focus source, but SECONDARY-only --
    /// exalting it onto a FINGERS-only ring would leave the ring with
    /// nowhere it could still be worn (empty slot intersection), which
    /// must exclude it outright, not just warn about it.
    #[test]
    fn a_source_with_no_overlapping_slot_is_illegal() {
        let candidates = exalt_candidates("Brass_Ring", "focus", &HashMap::new(), &[], None);
        assert!(
            !candidates.iter().any(|c| c.name == "A Shimmering Orb"),
            "a SECONDARY-only source must not be offered for a FINGERS-only ring"
        );
    }

    /// A candidate legitimately usable in the target's slot, but by a
    /// class the player isn't actually running, must not be offered --
    /// "relevant pieces" means relevant to the player, not just legal in
    /// the abstract. "Band of Screaming Winds" (NEC-only click, FINGERS)
    /// vs. a Warrior-only active roster.
    #[test]
    fn a_source_usable_by_a_different_class_than_the_player_is_excluded() {
        // why: "All" era -- isolating the class filter, not the (separately
        // tested) era one; Berserkers Ring is real but a later era than
        // the default CURRENT_ERA ceiling would allow through.
        let candidates = exalt_candidates(
            "Brass_Ring",
            "click",
            &HashMap::new(),
            &["Warrior".to_string()],
            Some("All"),
        );
        assert!(!candidates.iter().any(|c| c.name == "Band of Screaming Winds"));
        // "Berserkers Ring" -- WAR is one of its classes -- must still show up.
        assert!(candidates.iter().any(|c| c.name == "Berserkers Ring"));
    }

    #[test]
    fn an_item_is_never_offered_as_its_own_exaltation_source() {
        let candidates = exalt_candidates("Adamantite_Band", "focus", &HashMap::new(), &[], None);
        assert!(!candidates.iter().any(|c| c.name == "Adamantite Band"));
    }

    #[test]
    fn an_unknown_socket_key_returns_nothing() {
        assert!(exalt_candidates("Brass_Ring", "not-a-real-socket", &HashMap::new(), &[], None).is_empty());
    }

    #[test]
    fn ornament_has_no_candidates_at_all() {
        // Ornamentation takes a cosmetic token, not an exaltable effect --
        // EXALT_SLOTS carries no effect_key for it, so there's nothing to
        // source from the item catalog.
        assert!(exalt_candidates("Brass_Ring", "ornament", &HashMap::new(), &[], None).is_empty());
    }

    /// End to end: socketing a real source's effect into a target with no
    /// native effect of its own must show the *source's* effect, not a
    /// bare "open" socket.
    #[test]
    fn item_with_exalts_shows_the_socketed_sources_own_effect() {
        let mut assignments = HashMap::new();
        assignments.insert("focus".to_string(), "Adamantite_Band".to_string());
        let dto = item_with_exalts("Brass_Ring", 5, &assignments).expect("real catalog id");
        let focus = dto.exalts.iter().find(|e| e.key == "focus").unwrap();
        assert!(focus.unlocked, "tier 5 is well past focus's own +1 requirement");
        assert_eq!(
            focus.effect.as_ref().map(|e| e.name.as_str()),
            Some("Summoning Haste I"),
            "should carry Adamantite Band's own effect, not Brass Ring's (it has none)"
        );
    }

    /// A socket with no assignment still falls back to the item's own
    /// native effect (or "open" if it has none) -- assigning one socket
    /// must not disturb the others.
    #[test]
    fn unassigned_sockets_still_show_the_items_own_native_effect() {
        let mut assignments = HashMap::new();
        assignments.insert("focus".to_string(), "Adamantite_Band".to_string());
        // "Berserkers Ring" has a real native click effect of its own.
        let dto = item_with_exalts("Berserkers_Ring", 5, &assignments).expect("real catalog id");
        let click = dto.exalts.iter().find(|e| e.key == "click").unwrap();
        assert_eq!(click.effect.as_ref().map(|e| e.name.as_str()), Some("Firefist"));
    }
}
