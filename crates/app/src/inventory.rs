//! why: parses `/outputfile inventory` dumps into `gearplanner::SLOTS`
//!
//! The dump itself never appears in the log stream -- `outputfile.
//! complete` only confirms it finished writing. Lands in `AppConfig::
//! base_dir`, one level above `Logs` -- why `AppConfig` stores the base
//! folder. Frontend reaches this over IPC, not a file.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// why: `tier` is the "+N" the game prints, read directly off the dump
#[derive(Debug, Clone, serde::Serialize)]
pub struct InventoryItem {
    pub name: String,
    pub tier: u8,
}

/// why: dump `Location` -> planner slot keys, confirmed against a real
/// dump. 4 locations appear twice each (Ear/Wrist/Fingers/Any Slot),
/// mapped by occurrence order -- no other way to distinguish them.
/// Every other dump location (Bank/General/SharedBank/...) is skipped,
/// real inventory but no paper-doll slot for it.
const SLOT_ORDER: &[(&str, &[&str])] = &[
    ("Ear", &["EAR1", "EAR2"]),
    ("Head", &["HEAD"]),
    ("Face", &["FACE"]),
    ("Neck", &["NECK"]),
    ("Shoulders", &["SHOULDERS"]),
    ("Arms", &["ARMS"]),
    ("Back", &["BACK"]),
    ("Wrist", &["WRIST1", "WRIST2"]),
    ("Range", &["RANGE"]),
    ("Hands", &["HANDS"]),
    ("Primary", &["PRIMARY"]),
    ("Secondary", &["SECONDARY"]),
    ("Fingers", &["FINGER1", "FINGER2"]),
    ("Chest", &["CHEST"]),
    ("Legs", &["LEGS"]),
    ("Feet", &["FEET"]),
    ("Waist", &["WAIST"]),
    ("Ammo", &["AMMO"]),
    ("Any Slot", &["ANY1", "ANY2"]),
];

/// why: splits "Name +N" -> (name, N), 0 if untiered; pub(crate) so
/// `raiding.rs` can match a tiered loot line against the wiki's untiered entry
pub(crate) fn strip_tier(name: &str) -> (&str, u8) {
    if let Some((base, tail)) = name.rsplit_once(" +") {
        if let Ok(n) = tail.parse::<u8>() {
            return (base, n);
        }
    }
    (name, 0)
}

/// why: raw dump location -> a label a player actually recognizes.
/// Numbered by the same bag/bank index the dump itself uses, not the
/// bag's own item name (e.g. "Spacious Rucksack") -- that's a real,
/// separate cross-reference this doesn't attempt, and EQ players
/// already talk about their own bags by slot number ("check gen 3").
fn display_location(location: &str) -> String {
    if let Some(rest) = location.strip_prefix("General ") {
        return match rest.split_once("-Slot") {
            Some((bag, slot)) => format!("General bag {bag}, slot {slot}"),
            None => format!("General bag {rest} (the bag itself)"),
        };
    }
    if let Some(rest) = location.strip_prefix("Bank") {
        return match rest.split_once("-Slot") {
            Some((bag, slot)) => format!("Bank bag {bag}, slot {slot}"),
            None if rest.is_empty() => "Bank".to_string(),
            None => format!("Bank bag {rest} (the bag itself)"),
        };
    }
    if let Some(slot) = location.strip_prefix("SharedBank-Slot") {
        return format!("Shared Bank, slot {slot}");
    }
    if location == "SharedBank" {
        return "Shared Bank".to_string();
    }
    if let Some(slot) = location.strip_prefix("Personal-Depot") {
        return format!("Personal Depot, slot {slot}");
    }
    if location == "KeyRing" {
        return "Key Ring".to_string();
    }
    // why: an equip-doll slot -- everything else this function doesn't
    // recognize is shown as-is, but a bare slot name ("Chest") reads
    // like a location by coincidence only; spelling out "Equipped"
    // makes it unambiguous
    if SLOT_ORDER.iter().any(|(loc, _)| *loc == location) {
        return format!("Equipped ({location})");
    }
    location.to_string()
}

/// why: raw dump location -> which real container it belongs to, for
/// the browser view -- a separate, independent function from
/// `display_location` (not built from a shared helper) since the two
/// have genuinely different phrasing needs ("Equipped (Fingers)" as one
/// string vs. a container/slot pair to group by) and `display_location`
/// is already shipped/tested; not worth the coupling to unify them.
fn container_key(location: &str) -> String {
    if let Some(rest) = location.strip_prefix("General ") {
        return format!(
            "General bag {}",
            rest.split_once("-Slot").map_or(rest, |(bag, _)| bag)
        );
    }
    if let Some(rest) = location.strip_prefix("Bank") {
        if rest.is_empty() {
            return "Bank".to_string();
        }
        return format!(
            "Bank bag {}",
            rest.split_once("-Slot").map_or(rest, |(bag, _)| bag)
        );
    }
    if location.starts_with("SharedBank") {
        return "Shared Bank".to_string();
    }
    if location.starts_with("Personal-Depot") {
        return "Personal Depot".to_string();
    }
    if location == "KeyRing" {
        return "Key Ring".to_string();
    }
    location.to_string()
}

/// why: this row's own label *within* its container -- None means the
/// row itself IS the container (a bag's own header row), see `parse`'s
/// use of this alongside `container_key`
fn slot_label(location: &str) -> Option<String> {
    if let Some(rest) = location.strip_prefix("General ") {
        return rest
            .split_once("-Slot")
            .map(|(_, slot)| format!("slot {slot}"));
    }
    if let Some(rest) = location.strip_prefix("Bank") {
        return rest
            .split_once("-Slot")
            .map(|(_, slot)| format!("slot {slot}"));
    }
    // why: flat -- "SharedBank1"/"Personal-Depot7", no bag-index level
    // and no "-Slot" separator at all, confirmed against the real dump
    // (unlike General/Bank, which nest a bag index then "-SlotN")
    if let Some(slot) = location.strip_prefix("SharedBank") {
        return Some(format!("slot {slot}"));
    }
    if let Some(slot) = location.strip_prefix("Personal-Depot") {
        return Some(format!("slot {slot}"));
    }
    None
}

/// why: true only for a bag/bank's own bare container-defining row
/// ("General 1", "Bank5", bare "Bank") -- a bag's own numbered CONTENTS
/// aren't sockets of the bag, they're real, distinct items, each of
/// which (equipped, bagged, shared-banked, depot, or the bag/pouch item
/// itself) can legitimately carry its own nested exalt/augment socket.
/// An exclusion, not an enumeration -- deliberately, so a container type
/// this doesn't know about (Dragon's Hoard, if it ever shows up) is
/// eligible by default rather than silently swallowed the way this bug
/// class already proved happens to an un-enumerated case.
fn is_bag_container_header(base: &str) -> bool {
    if let Some(rest) = base.strip_prefix("General ") {
        return !rest.contains("-Slot");
    }
    if let Some(rest) = base.strip_prefix("Bank") {
        return !rest.contains("-Slot");
    }
    false
}

/// why: `Vec`, not a `HashMap` -- containers display in first-seen
/// (dump) order, and there are only ever a few dozen of them, so a
/// linear find is cheap and simpler than keeping a parallel index
fn container_mut<'a>(
    containers: &'a mut Vec<InventoryContainerDto>,
    label: &str,
) -> &'a mut InventoryContainerDto {
    if let Some(i) = containers.iter().position(|c| c.label == label) {
        &mut containers[i]
    } else {
        containers.push(InventoryContainerDto {
            label: label.to_string(),
            bag_item: None,
            slots: Vec::new(),
        });
        containers.last_mut().expect("just pushed")
    }
}

/// why: strips any trailing "-Slot<N>", not just modelled ones -- see `parse`'s doc
fn strip_trailing_numbered_slot(location: &str) -> Option<&str> {
    let (base, tail) = location.rsplit_once("-Slot")?;
    tail.parse::<u32>().ok()?;
    Some(base)
}

/// why: `-Slot<N>` -> exalt socket type, for the 4 confirmed socket
/// numbers (7=focus, 8=click confirmed via real filled examples against
/// `packs/items.json`; 9/10 inferred by `gearplanner::EXALT_SLOTS`'
/// order, not independently confirmed). `ornament` deliberately not
/// modelled -- its dump slot number isn't even consistent.
const EXALT_SOCKET_SUFFIXES: &[(&str, &str)] = &[
    ("-Slot7", "focus"),
    ("-Slot8", "click"),
    ("-Slot9", "worn"),
    ("-Slot10", "proc"),
];

/// why: exalt source's display name always carries this literal suffix
const EXALTATION_SUFFIX: &str = " (Exaltation)";

/// why: one real copy's own resting place -- "the locate feature"'s
/// entire payload, keyed externally by item name in `ParsedInventory::
/// locations` the same way `owned`/`owned_tier` already are
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct InventoryLocation {
    /// why: player-recognizable, not the raw dump string -- see `display_location`
    pub label: String,
    pub tier: u8,
    pub count: u32,
}

/// why: one real item sitting in a storage container -- the browser
/// view's own per-row payload, `slot` already player-recognizable
#[derive(Debug, Clone, serde::Serialize)]
pub struct InventorySlotDto {
    pub slot: String,
    pub item: String,
    pub tier: u8,
    pub count: u32,
}

/// why: one real storage container (a bag, the bank, the depot, key
/// ring, ...) -- the browser view groups by this, `locations`/`owned`
/// don't need to
#[derive(Debug, Clone, serde::Serialize)]
pub struct InventoryContainerDto {
    pub label: String,
    /// why: the bag/pouch's own name when it's a real numbered container
    /// (e.g. "Spacious Rucksack") -- None for containers with no such
    /// concept in this dump (Shared Bank, Personal Depot, Key Ring)
    pub bag_item: Option<String>,
    pub slots: Vec<InventorySlotDto>,
}

/// why: both halves of a dump read in one pass -- equipped, and total owned
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ParsedInventory {
    pub equipped: HashMap<String, InventoryItem>,
    /// why: total copies owned, `Count` summed across every row; excludes
    /// exalt-socket rows -- socketed isn't a spare copy
    pub owned: HashMap<String, u32>,
    /// why: highest tier owned of a name, since only the best copy matters
    pub owned_tier: HashMap<String, u8>,
    /// why: equip slot -> (socket -> source item), real ground truth
    /// unlike `Ingest::exaltation_procs`' proc-evidence inference; no
    /// entry for an empty socket
    pub exalted: HashMap<String, HashMap<String, String>>,
    /// why: every real copy's own row, not just the summed total --
    /// "where is my X" (the locate feature) needs each individual
    /// resting place, `owned` only ever needed the sum. Same key space
    /// (tier-stripped name) and same row set as `owned`/`owned_tier` --
    /// built inline in the same loop, not a second pass.
    pub locations: HashMap<String, Vec<InventoryLocation>>,
    /// why: the browser view -- storage only (bags/bank/depot/key ring),
    /// deliberately excludes equip-doll slots (already the Gear
    /// Planner's own job). First-seen order, which is the dump's own
    /// order -- no reason to re-sort what the game already lays out sensibly.
    pub containers: Vec<InventoryContainerDto>,
}

impl ParsedInventory {
    /// why: case-insensitive -- callers (GdLink's own "locate" affordance)
    /// pass a wiki-spelled name, not guaranteed byte-for-byte identical to
    /// the log's own casing, same reasoning skyquests.rs's owned_ci
    /// lookup already applies to this exact same data
    pub fn locate(&self, name: &str) -> &[InventoryLocation] {
        self.locations
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_slice())
            .unwrap_or(&[])
    }
}

/// why: tab-separated dump (Location/Name/ID/Count/Slots); `equipped`
/// only from `SLOT_ORDER`, `owned` sums every non-exalt-socket row.
///
/// A bag item's own nested augment sockets look structurally identical
/// to an equip-doll slot's exalt sockets (both a trailing `-Slot<N>`).
/// Distinguished by walking in order, remembering the last base row's
/// location and (if real) its equip key -- a socket row always
/// immediately follows the row it belongs to, so this pair is enough to
/// tell a real exalt socket from a bag item's nested one without
/// misattributing to whatever equip slot came earlier in the file.
pub fn parse(path: &Path) -> std::io::Result<ParsedInventory> {
    let text = std::fs::read_to_string(path)?;
    let mut seen: HashMap<&str, usize> = HashMap::new();
    let mut out = ParsedInventory::default();
    let mut last_base_location: &str = "";
    let mut last_equip_key: Option<&'static str> = None;

    for line in text.lines().skip(1) {
        let mut cols = line.split('\t');
        let (Some(location), Some(name), Some(_id), Some(count)) =
            (cols.next(), cols.next(), cols.next(), cols.next())
        else {
            continue;
        };
        // why: real bug, caught while building the locate feature --
        // confirmed against a real dump: "KeyRing\tName\tID\t" (a
        // section's own re-embedded column header, trailing tab makes
        // it 4 fields) parses as if it owns 1 copy of an item literally
        // named "Name" at location "KeyRing". Harmless today (nothing
        // real is ever named "Name"), but the locate feature makes any
        // owned-item listing directly visible, where this would show
        // up as nonsense. A genuine Count field is never empty; this is.
        if count.is_empty() {
            continue;
        }

        // why: checked structurally against every "-Slot<N>" row, not
        // just the 4 modelled suffixes -- an unmodelled socket (e.g.
        // ornament) used to fall through and corrupt last_base_location/
        // last_equip_key, breaking matching for its real siblings
        if let Some(base) = strip_trailing_numbered_slot(location) {
            // why: real bug, caught empirically against the live reference
            // dump while building the locate feature -- `base ==
            // last_base_location` alone treats a bag's OWN bare name
            // ("Bank5") as if it could carry sockets the same way an
            // equip-doll slot ("Back") can, so the bag's *first* real
            // content row ("Bank5-Slot1") -- and every row after it,
            // since a matched continuation never advances
            // last_base_location -- gets silently swallowed as a
            // "socket" instead of counted, until some later row happens
            // to break the false match by coincidence. Confirmed: real
            // items (Efreeti War Club, Ceremonial Belt, White Dragon
            // Scales x4, Belt of the Four Winds, Blood-Drawn Runes, ...)
            // vanished from `owned` entirely in the live dump. Excludes
            // only a bag/bank's own bare container-defining row -- every
            // other real item (equipped, bagged, in the shared bank, in
            // the depot, or the bag/pouch item itself) legitimately can
            // carry its own nested exalt/augment socket.
            if base == last_base_location && !is_bag_container_header(base) {
                if name != "Empty" {
                    if let (Some(equip_key), Some(socket_key)) = (
                        last_equip_key,
                        EXALT_SOCKET_SUFFIXES
                            .iter()
                            .find(|(suffix, _)| location.ends_with(suffix))
                            .map(|&(_, k)| k),
                    ) {
                        let source_name = name.strip_suffix(EXALTATION_SUFFIX).unwrap_or(name);
                        out.exalted
                            .entry(equip_key.to_string())
                            .or_default()
                            .insert(socket_key.to_string(), source_name.to_string());
                    }
                }
                continue;
            }
        }

        last_base_location = location;
        last_equip_key = None;

        if name == "Empty" {
            continue;
        }
        let (base_name, tier) = strip_tier(name);
        let count: u32 = count.parse().unwrap_or(1);
        *out.owned.entry(base_name.to_string()).or_insert(0) += count;
        let best_tier = out.owned_tier.entry(base_name.to_string()).or_insert(0);
        *best_tier = (*best_tier).max(tier);
        out.locations
            .entry(base_name.to_string())
            .or_default()
            .push(InventoryLocation {
                label: display_location(location),
                tier,
                count,
            });

        // why: the browser view (bags/bank/depot/key ring), grouped by
        // real container -- deliberately excludes equip-doll slots,
        // already served by the Gear Planner/Character Sheet, so this
        // stays scoped to storage. A container's own header row (e.g.
        // "General 1" naming "Spacious Rucksack") sets `bag_item`
        // instead of adding a slot -- it's the bag's own identity, not
        // something sitting inside it.
        if !SLOT_ORDER.iter().any(|(loc, _)| *loc == location) {
            let container = container_mut(&mut out.containers, &container_key(location));
            match slot_label(location) {
                Some(slot) => container.slots.push(InventorySlotDto {
                    slot,
                    item: base_name.to_string(),
                    tier,
                    count,
                }),
                None => container.bag_item = Some(base_name.to_string()),
            }
        }

        let Some((_, slot_keys)) = SLOT_ORDER.iter().find(|(loc, _)| *loc == location) else {
            continue;
        };
        let idx = seen.entry(location).or_insert(0);
        let Some(&key) = slot_keys.get(*idx) else {
            // why: more rows than SLOT_ORDER expects -- skip overflow, don't misassign
            continue;
        };
        *idx += 1;
        last_equip_key = Some(key);
        out.equipped.insert(
            key.to_string(),
            InventoryItem {
                name: base_name.to_string(),
                tier,
            },
        );
    }
    Ok(out)
}

/// why: `file` trusted only as a bare filename -- a stray `..` can't escape `base_dir`
pub fn dump_path(base_dir: &Path, file: &str) -> std::io::Result<PathBuf> {
    let name_only = Path::new(file).file_name().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("not a plain filename: {file}"),
        )
    })?;
    Ok(base_dir.join(name_only))
}

/// why: `/outputfile` covers more than inventory -- narrows to dumps this module can read
pub fn is_inventory_dump(file: &str) -> bool {
    file.ends_with("-Inventory.txt")
}

/// why: read off the filename, not looked up -- best-effort, not guaranteed
pub fn inventory_character(file: &str) -> Option<String> {
    file.split('_')
        .next()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// why: most recent dump already on disk, offered on startup not just fresh dumps
pub fn find_existing_dump(base_dir: &Path) -> Option<(String, Option<String>)> {
    let entries = std::fs::read_dir(base_dir).ok()?;
    let newest = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_str().is_some_and(is_inventory_dump))
        .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok())?;
    let file = newest.file_name().to_string_lossy().into_owned();
    let character = inventory_character(&file);
    Some((file, character))
}

#[cfg(test)]
mod parse_tests {
    use super::*;

    fn scratch_file(name: &str, text: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("eqlp-inv-parse-{name}-{}.txt", std::process::id()));
        std::fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn owned_tier_keeps_the_highest_tier_seen_for_a_name() {
        // why: two copies at different tiers -- owned_tier must report the best, not the last row
        let path = scratch_file(
            "tier",
            "Location\tName\tID\tCount\tSlots\n\
             General 1-Slot1\tBrass Ring +3\t100\t1\t10\n\
             General 1-Slot2\tBrass Ring +7\t100\t1\t10\n\
             General 1-Slot3\tBrass Ring\t100\t1\t10\n",
        );
        let parsed = parse(&path).unwrap();
        assert_eq!(
            parsed.owned.get("Brass Ring"),
            Some(&3),
            "three copies, tier-stripped, summed"
        );
        assert_eq!(
            parsed.owned_tier.get("Brass Ring"),
            Some(&7),
            "the best of the three copies"
        );
    }

    /// why: real unedited lines, Back's two socketed exaltations, focus + click
    #[test]
    fn real_exaltation_sockets_resolve_to_their_confirmed_socket_keys() {
        let path = scratch_file(
            "exalt",
            "Location\tName\tID\tCount\tSlots\n\
             Back\tShield of the Immaculate +3\t11551\t1\t10\n\
             Back-Slot2\tEmpty\t0\t0\t0\n\
             Back-Slot7\tWhite Dragonscale Cloak (Exaltation)\t11603\t1\t10\n\
             Back-Slot8\tShield of the Immaculate (Exaltation)\t11551\t1\t10\n\
             Back-Slot9\tEmpty\t0\t0\t0\n",
        );
        let parsed = parse(&path).unwrap();
        let back = parsed
            .exalted
            .get("BACK")
            .expect("Back should have exalt sockets recorded");
        assert_eq!(
            back.get("focus"),
            Some(&"White Dragonscale Cloak".to_string()),
            "Slot7 -- confirmed focus-effect item"
        );
        assert_eq!(
            back.get("click"),
            Some(&"Shield of the Immaculate".to_string()),
            "Slot8 -- confirmed click-effect item"
        );
        assert_eq!(
            back.len(),
            2,
            "the two Empty sockets (2 and 9) shouldn't produce entries"
        );
    }

    /// why: two Ear slots, each own exalt socket -- must land EAR1/EAR2, not merge
    #[test]
    fn two_copies_of_the_same_doll_slot_keep_independent_exalt_sockets() {
        let path = scratch_file(
            "ear",
            "Location\tName\tID\tCount\tSlots\n\
             Ear\tEarring of Displacement +2\t14559\t1\t10\n\
             Ear-Slot7\tEmpty\t0\t0\t0\n\
             Ear-Slot8\tEarring of Displacement (Exaltation)\t14559\t1\t10\n\
             Ear\tIvandyr's Hoop +5\t10082\t1\t10\n\
             Ear-Slot7\tEmpty\t0\t0\t0\n\
             Ear-Slot9\tEmpty\t0\t0\t0\n",
        );
        let parsed = parse(&path).unwrap();
        assert_eq!(
            parsed.exalted.get("EAR1").unwrap().get("click"),
            Some(&"Earring of Displacement".to_string())
        );
        assert!(
            !parsed.exalted.contains_key("EAR2"),
            "the second Ear had nothing real socketed"
        );
    }

    /// why: a bag item's nested sockets look identical structurally --
    /// must not be misattributed to an earlier equip-doll slot
    #[test]
    fn a_bagged_items_own_nested_exalt_sockets_are_not_attributed_to_an_equip_slot() {
        let path = scratch_file(
            "bag",
            "Location\tName\tID\tCount\tSlots\n\
             Back\tShield of the Immaculate +3\t11551\t1\t10\n\
             Back-Slot7\tWhite Dragonscale Cloak (Exaltation)\t11603\t1\t10\n\
             General 1\tSpacious Rucksack\t177751\t1\t24\n\
             General 1-Slot3\tInsidious Robe +4\t1247\t1\t10\n\
             General 1-Slot3-Slot7\tSome Other Exaltation (Exaltation)\t9999\t1\t10\n",
        );
        let parsed = parse(&path).unwrap();
        let back = parsed
            .exalted
            .get("BACK")
            .expect("Back's own real exalt socket");
        assert_eq!(
            back.len(),
            1,
            "only Back's own real socket, not the bag item's nested one too"
        );
        assert!(
            !parsed.exalted.contains_key("General 1"),
            "a bag is never an equip-doll slot"
        );
    }

    /// why: a socketed source must never inflate `owned` -- consumed, not spare
    #[test]
    fn a_socketed_exaltation_source_does_not_count_toward_owned() {
        let path = scratch_file(
            "notowned",
            "Location\tName\tID\tCount\tSlots\n\
             Back\tShield of the Immaculate +3\t11551\t1\t10\n\
             Back-Slot7\tWhite Dragonscale Cloak (Exaltation)\t11603\t1\t10\n",
        );
        let parsed = parse(&path).unwrap();
        assert_eq!(parsed.owned.get("White Dragonscale Cloak"), None);
        assert_eq!(
            parsed.owned.get("White Dragonscale Cloak (Exaltation)"),
            None
        );
    }

    /// why: real lines from the live reference dump -- the locate
    /// feature's whole point, one row per real resting place
    #[test]
    fn locations_read_a_real_bag_bank_depot_and_equip_slot() {
        let path = scratch_file(
            "locations",
            "Location\tName\tID\tCount\tSlots\n\
             General 1-Slot2\tBlade of Abrogation +1\t5430\t1\t10\n\
             Bank5-Slot4\tBlade of Abrogation +2\t5430\t1\t10\n\
             Personal-Depot7\tAmber\t10022\t31\t10\n\
             Fingers\tRing of Pureblood +5\t1540\t1\t10\n",
        );
        let parsed = parse(&path).unwrap();
        let blade = parsed.locate("Blade of Abrogation");
        assert_eq!(blade.len(), 2, "one copy in the bag, one in the bank");
        assert!(blade
            .iter()
            .any(|l| l.label == "General bag 1, slot 2" && l.tier == 1));
        assert!(blade
            .iter()
            .any(|l| l.label == "Bank bag 5, slot 4" && l.tier == 2));
        assert_eq!(
            parsed.locate("Amber"),
            &[InventoryLocation {
                label: "Personal Depot, slot 7".to_string(),
                tier: 0,
                count: 31,
            }]
        );
        assert_eq!(
            parsed.locate("ring of pureblood"), // why: case-insensitive, see locate's own doc
            &[InventoryLocation {
                label: "Equipped (Fingers)".to_string(),
                tier: 5,
                count: 1,
            }]
        );
        assert!(parsed.locate("Nothing Owned").is_empty());
    }

    /// why: real bug, caught while building the locate feature -- see
    /// this guard's own doc in `parse` for the full real-dump line
    #[test]
    fn a_reembedded_section_header_row_is_not_mistaken_for_an_owned_item() {
        let path = scratch_file(
            "keyring-header",
            "Location\tName\tID\tCount\tSlots\n\
             KeyRing\tName\tID\t\n",
        );
        let parsed = parse(&path).unwrap();
        assert!(!parsed.owned.contains_key("Name"));
        assert!(parsed.locate("Name").is_empty());
    }

    /// why: real bug, caught empirically against the live reference dump
    /// (not guessed) while building the locate feature -- `base ==
    /// last_base_location` alone treated a bag's own bare name as if it
    /// could carry sockets the same as an equip-doll slot, silently
    /// swallowing a bag's first real content row and (since a matched
    /// continuation never advances last_base_location) every row after
    /// it too, until something coincidentally broke the false match.
    /// These are the exact real lines that exposed it: Efreeti War Club
    /// (Bank5's very first content row) and Ceremonial Belt (General
    /// 1's) both vanished from `owned` entirely before the fix.
    #[test]
    fn a_bags_own_first_content_slot_is_a_real_item_not_a_socket_of_the_bag() {
        let path = scratch_file(
            "bag-first-slot",
            "Location\tName\tID\tCount\tSlots\n\
             Bank5\tDriftwood Treasure Chest\t17406\t1\t10\n\
             Bank5-Slot1\tEfreeti War Club +1\t20845\t1\t10\n\
             Bank5-Slot1-Slot2\tEmpty\t0\t0\t0\n\
             Bank5-Slot1-Slot7\tEmpty\t0\t0\t0\n\
             Bank5-Slot2\tEfreeti Magi Staff +1\t20870\t1\t10\n\
             General 1\tSpacious Rucksack\t177751\t1\t24\n\
             General 1-Slot1\tCeremonial Belt\t20838\t1\t10\n",
        );
        let parsed = parse(&path).unwrap();
        assert_eq!(parsed.owned.get("Efreeti War Club"), Some(&1));
        assert_eq!(
            parsed.locate("Efreeti War Club"),
            &[InventoryLocation {
                label: "Bank bag 5, slot 1".to_string(),
                tier: 1,
                count: 1,
            }]
        );
        // why: the row after the swallowed one was already correct by
        // coincidence (its own preceding Empty socket rows happened to
        // break the false match) -- still asserted, so a future change
        // can't silently re-break the general case while this specific
        // regression stays green
        assert_eq!(parsed.owned.get("Efreeti Magi Staff"), Some(&1));
        assert_eq!(parsed.owned.get("Ceremonial Belt"), Some(&1));
        assert_eq!(
            parsed.locate("Ceremonial Belt"),
            &[InventoryLocation {
                label: "General bag 1, slot 1".to_string(),
                tier: 0,
                count: 1,
            }]
        );
        // why: the bag containers themselves are real owned items too
        assert_eq!(parsed.owned.get("Driftwood Treasure Chest"), Some(&1));
        assert_eq!(parsed.owned.get("Spacious Rucksack"), Some(&1));

        // why: the browser view must show the same real slots, grouped
        // by container, bag's own name captured separately from its
        // contents -- this is the same real data, viewed the other way
        let bank5 = parsed
            .containers
            .iter()
            .find(|c| c.label == "Bank bag 5")
            .expect("Bank bag 5 present");
        assert_eq!(bank5.bag_item.as_deref(), Some("Driftwood Treasure Chest"));
        assert_eq!(
            bank5.slots.len(),
            2,
            "Efreeti War Club + Efreeti Magi Staff, not their empty sockets"
        );
        assert!(bank5
            .slots
            .iter()
            .any(|s| s.slot == "slot 1" && s.item == "Efreeti War Club" && s.tier == 1));

        let general1 = parsed
            .containers
            .iter()
            .find(|c| c.label == "General bag 1")
            .expect("General bag 1 present");
        assert_eq!(general1.bag_item.as_deref(), Some("Spacious Rucksack"));
        assert_eq!(general1.slots.len(), 1);
        assert_eq!(general1.slots[0].slot, "slot 1");
        assert_eq!(general1.slots[0].item, "Ceremonial Belt");
    }

    /// why: browser view must never include equip-doll slots -- those
    /// are the Gear Planner/Character Sheet's own job, not this one's
    #[test]
    fn the_browser_view_excludes_equipped_items() {
        let path = scratch_file(
            "browser-excludes-equipped",
            "Location\tName\tID\tCount\tSlots\n\
             Fingers\tRing of Pureblood +5\t1540\t1\t10\n\
             General 1\tSpacious Rucksack\t177751\t1\t24\n\
             General 1-Slot1\tCeremonial Belt\t20838\t1\t10\n",
        );
        let parsed = parse(&path).unwrap();
        assert!(parsed.equipped.contains_key("FINGER1"));
        assert!(
            !parsed
                .containers
                .iter()
                .any(|c| c.slots.iter().any(|s| s.item == "Ring of Pureblood")),
            "equipped items belong to `equipped`, not `containers`"
        );
        assert_eq!(parsed.containers.len(), 1, "only the real bag shows up");
    }

    /// why: real lines -- Shared Bank and Personal Depot are flat
    /// (a bare number appended directly, e.g. "SharedBank1"), no
    /// bag-index level and no "-Slot" separator at all, unlike General/
    /// Bank's two-level "bag index, then -SlotN" scheme. A naive port of
    /// that scheme would either miss these slots entirely or, worse,
    /// wrongly treat a real item's own nested augment socket as if it
    /// couldn't exist here (the same bug class this whole fix is about,
    /// just for a container type this dump's real data never exercises).
    #[test]
    fn shared_bank_and_personal_depot_use_their_own_flat_numbering() {
        let path = scratch_file(
            "flat-containers",
            "Location\tName\tID\tCount\tSlots\n\
             SharedBank1\tEfreeti War Club +1\t20845\t1\t10\n\
             SharedBank1-Slot7\tSome Focus Item (Exaltation)\t1\t1\t10\n\
             Personal-Depot7\tAmber\t10022\t31\t10\n",
        );
        let parsed = parse(&path).unwrap();
        let shared = parsed
            .containers
            .iter()
            .find(|c| c.label == "Shared Bank")
            .expect("Shared Bank present");
        assert_eq!(
            shared.slots.len(),
            1,
            "the nested socket is not a second item"
        );
        assert_eq!(shared.slots[0].slot, "slot 1");
        assert_eq!(shared.slots[0].item, "Efreeti War Club");
        // why: the nested socket is real evidence too, just not counted
        // as a separate owned item -- confirms it wasn't simply dropped
        assert!(!parsed.owned.contains_key("Some Focus Item"));
        assert!(!parsed.owned.contains_key("Some Focus Item (Exaltation)"));

        let depot = parsed
            .containers
            .iter()
            .find(|c| c.label == "Personal Depot")
            .expect("Personal Depot present");
        assert_eq!(depot.slots.len(), 1);
        assert_eq!(depot.slots[0].slot, "slot 7");
        assert_eq!(depot.slots[0].count, 31);
    }
}

#[cfg(test)]
mod find_existing_dump_tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("eqlp-inv-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(path: &Path, when: SystemTime) {
        std::fs::write(path, b"").unwrap();
        let f = std::fs::File::open(path).unwrap();
        f.set_modified(when).unwrap();
    }

    #[test]
    fn picks_the_most_recently_modified_dump_and_ignores_other_files() {
        let dir = scratch_dir("newest");
        let now = SystemTime::now(); // clock-exempt: test, touches real file mtimes on purpose
        touch(
            &dir.join("Manipulator_rivervale-Inventory.txt"),
            now - Duration::from_secs(60),
        );
        touch(&dir.join("Thiole_neriak-Inventory.txt"), now);
        touch(&dir.join("dbg.txt"), now); // not a dump, must not win even with the same mtime

        let (file, character) = find_existing_dump(&dir).expect("a dump exists");
        assert_eq!(file, "Thiole_neriak-Inventory.txt");
        assert_eq!(character.as_deref(), Some("Thiole"));
    }

    #[test]
    fn none_when_the_folder_has_no_dump_at_all() {
        let dir = scratch_dir("empty");
        std::fs::write(dir.join("eqlog_Someone_zone.txt"), b"").unwrap();
        assert!(find_existing_dump(&dir).is_none());
    }
}
