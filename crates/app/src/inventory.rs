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

        // why: checked structurally against every "-Slot<N>" row, not
        // just the 4 modelled suffixes -- an unmodelled socket (e.g.
        // ornament) used to fall through and corrupt last_base_location/
        // last_equip_key, breaking matching for its real siblings
        if let Some(base) = strip_trailing_numbered_slot(location) {
            if base == last_base_location {
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
