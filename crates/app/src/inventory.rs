//! Parsing `/outputfile inventory` dumps into the gear planner's own slot
//! vocabulary (`gearplanner::SLOTS`).
//!
//! The dump itself is never in the log stream -- `outputfile.complete`
//! (`packs/eql.toml`) only ever sees the client's one-line confirmation
//! that a dump finished writing, naming the file. The file lands in the
//! game's base install folder (`AppConfig::base_dir`), one level above
//! `Logs`, which is the whole reason `AppConfig` was widened to store the
//! base folder instead of `Logs` directly -- see that module's doc.
//!
//! why: frontend reaches this over IPC (`get_inventory_dump`), not a file

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// One equipped item, as read from the dump. `tier` is the "+N" the game
/// itself prints on the item's name (0 if there isn't one) -- read
/// directly off the dump, not derived or guessed at.
#[derive(Debug, Clone, serde::Serialize)]
pub struct InventoryItem {
    pub name: String,
    pub tier: u8,
}

/// Maps the dump's own `Location` column values to the gear planner's slot
/// keys -- confirmed against a real dump (`~/eqlp/Manipulator_rivervale-
/// Inventory.txt`) matched up against the planner's actual `SLOTS` array,
/// not assumed to line up by name alone. Four locations
/// (`Ear`/`Wrist`/`Fingers`/`Any Slot`) each appear exactly twice in a
/// real dump, once per physical slot, confirmed by counting a real file
/// rather than assumed -- mapped by occurrence order (first row -> ...1,
/// second -> ...2), since the dump carries no other way to distinguish
/// them. Every other location the dump carries (Bank*/General
/// */SharedBank*/KeyRing/Activated/Augmentation/Equipment/Held) has no
/// entry here at all and is skipped -- real inventory, but not something
/// the gear planner's paper doll has a slot for.
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

/// `"Bloodstar Pendant +5"` -> `("Bloodstar Pendant", 5)`. A name with no
/// trailing `" +N"` (untiered gear, or gear that's never been upgraded)
/// returns the name unchanged with tier 0.
fn strip_tier(name: &str) -> (&str, u8) {
    if let Some((base, tail)) = name.rsplit_once(" +") {
        if let Ok(n) = tail.parse::<u8>() {
            return (base, n);
        }
    }
    (name, 0)
}

/// Both halves of a real `/outputfile inventory` dump -- what's equipped
/// (doll display) and how many of each name exist anywhere in the dump
/// (bags/bank/equipped, all summed). Deliberately one pass, one struct:
/// both are read off the same rows.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ParsedInventory {
    pub equipped: HashMap<String, InventoryItem>,
    /// Base (tier-stripped) item name -> total copies owned, every
    /// location summed. Each row is one real physical stack/item --
    /// `Count` is per-row stack size (high for reagents, always 1 for
    /// most gear, where a 2nd copy is instead a 2nd *row* in a different
    /// bag slot) -- so summing `Count` across every row of a name is the
    /// actual owned total either way, not an approximation.
    pub owned: HashMap<String, u32>,
    /// Base item name -> highest "+N" tier owned of it, anywhere in the
    /// dump. A player who owns several copies at different tiers only
    /// gets the best one back here -- for scoring/display purposes
    /// (`gearplanner::score_item`/`ItemDto::tier`), the best copy owned is
    /// the one that actually matters.
    pub owned_tier: HashMap<String, u8>,
}

/// Parses a real `/outputfile inventory` dump (tab-separated: `Location`,
/// `Name`, `ID`, `Count`, `Slots` -- confirmed against a real dump, not
/// assumed). `equipped` only ever comes from the bare `SLOT_ORDER`
/// locations (a sub-slot of a container, `Ear-Slot7`, an augment socket,
/// is skipped there); `owned` sums every non-empty row regardless of
/// location, since owning a copy in the bank counts same as owning it in
/// a bag.
pub fn parse(path: &Path) -> std::io::Result<ParsedInventory> {
    let text = std::fs::read_to_string(path)?;
    let mut seen: HashMap<&str, usize> = HashMap::new();
    let mut out = ParsedInventory::default();

    for line in text.lines().skip(1) {
        let mut cols = line.split('\t');
        let (Some(location), Some(name), Some(_id), Some(count)) =
            (cols.next(), cols.next(), cols.next(), cols.next())
        else {
            continue;
        };
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
            // More rows for this location than SLOT_ORDER expects for it --
            // an assumption this module makes turned out wrong for this
            // dump. Skip the overflow rather than silently misassigning it
            // to a slot it doesn't belong to.
            continue;
        };
        *idx += 1;
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

/// `<base_dir>/<file>`, with `file` trusted only as a bare filename -- it
/// comes straight from the client's own log line, which never contains a
/// directory component in practice, but this joins it under `base_dir`
/// rather than treating it as a path in its own right either way, so a
/// stray `..` in a malformed line can't escape `base_dir`. Used to resolve
/// a dump named by a fresh `outputfile.complete` line, and to build the
/// full path for whatever `find_existing_dump` finds already sitting on
/// disk.
pub fn dump_path(base_dir: &Path, file: &str) -> std::io::Result<PathBuf> {
    let name_only = Path::new(file).file_name().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("not a plain filename: {file}"),
        )
    })?;
    Ok(base_dir.join(name_only))
}

/// `/outputfile` covers more than just `inventory` (spawns, guildlist,
/// ...) -- this narrows down to dumps this module actually knows how to
/// read (`parse` only understands the inventory dump's own column
/// layout), so nothing ever offers to load a file that isn't one.
pub fn is_inventory_dump(file: &str) -> bool {
    file.ends_with("-Inventory.txt")
}

/// Read off the filename itself (`<Character>_<zone>-Inventory.txt`,
/// confirmed against a real dump), not looked up anywhere -- a best-effort
/// label, not a guaranteed-correct one.
pub fn inventory_character(file: &str) -> Option<String> {
    file.split('_')
        .next()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// The most recently written inventory dump already sitting in `base_dir`,
/// if any -- for offering to load one on startup/module-entry instead of
/// only ever reacting to a fresh `outputfile.complete` line. A player who
/// dumped their inventory last session, then closed and reopened the app,
/// has a real file on disk this whole time; nothing about that requires a
/// brand new dump before the app can use it.
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
        // Two copies of the same ring, at different tiers, in different
        // bags -- owned_tier must report the best one, not the last row.
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
        let now = SystemTime::now();
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
