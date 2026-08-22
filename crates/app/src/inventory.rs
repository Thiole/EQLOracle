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
/// returns the name unchanged with tier 0. `pub(crate)`: also reused by
/// `raiding.rs` to match a real loot line's own tiered item name ("You
/// looted an Engineer's Ring +4 from...") back against the wiki's
/// untiered drop-table entry ("Engineer's Ring") -- an exact-string
/// comparison between those two would otherwise never match at all,
/// silently hiding a real, confirmed drop.
pub(crate) fn strip_tier(name: &str) -> (&str, u8) {
    if let Some((base, tail)) = name.rsplit_once(" +") {
        if let Ok(n) = tail.parse::<u8>() {
            return (base, n);
        }
    }
    (name, 0)
}

/// `"Back-Slot7"` -> `Some("Back")`, `"General 1-Slot3"` -> `Some("General
/// 1")`, `"Back"` -> `None` -- structural, not tied to any particular
/// slot number (see `parse`'s own doc on why this has to catch *every*
/// numbered sub-slot the dump carries, not just the ones this module
/// goes on to actually model).
fn strip_trailing_numbered_slot(location: &str) -> Option<&str> {
    let (base, tail) = location.rsplit_once("-Slot")?;
    tail.parse::<u32>().ok()?;
    Some(base)
}

/// `<equip-slot location>-Slot<N>` -> the exaltation socket `N`
/// corresponds to, for the 4 real socket types that actually hold a
/// source item's effect -- confirmed empirically against a real dump,
/// not assumed: every filled example found (`White Dragonscale Cloak`/
/// `Rokyls Channelling Crystal`/`Robe of the Oracle`/`Ishva Mas Leggings`,
/// all in a real `-Slot7`) carries `focus` as their *own* effect type in
/// `packs/items.json`, and the one real `-Slot8` example
/// (`Shield of the Immaculate`) carries `click` -- matching `gearplanner
/// ::EXALT_SLOTS`' own declared order (ornament, focus, click, worn,
/// proc) exactly, so `-Slot9`/`-Slot10` are inferred as `worn`/`proc` by
/// that same order, not independently confirmed the same way (no filled
/// real example of either seen yet). The `ornament` slot itself is
/// deliberately not modelled here at all: its own dump slot number isn't
/// even consistent (`Head-Slot1`, `Back-Slot2`, absent entirely for
/// `Primary`) and no real filled example has been seen to confirm its
/// naming convention against.
const EXALT_SOCKET_SUFFIXES: &[(&str, &str)] = &[("-Slot7", "focus"), ("-Slot8", "click"), ("-Slot9", "worn"), ("-Slot10", "proc")];

/// A dump-reported exalt source's own display name always carries this
/// literal suffix (confirmed against every real example seen) -- the
/// underlying item is the same catalog entry either equipped or
/// exalted-in, just annotated to say which.
const EXALTATION_SUFFIX: &str = " (Exaltation)";

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
    /// actual owned total either way, not an approximation. Deliberately
    /// excludes exalt-socket rows (`exalted`, below) -- whatever's
    /// already socketed into a piece isn't a spare copy sitting free to
    /// use elsewhere, so counting it here would overstate what's
    /// actually available.
    pub owned: HashMap<String, u32>,
    /// Base item name -> highest "+N" tier owned of it, anywhere in the
    /// dump. A player who owns several copies at different tiers only
    /// gets the best one back here -- for scoring/display purposes
    /// (`gearplanner::score_item`/`ItemDto::tier`), the best copy owned is
    /// the one that actually matters.
    pub owned_tier: HashMap<String, u8>,
    /// Equip-doll slot key (`"BACK"`, `"EAR1"`, ...) -> (exalt socket key
    /// `"focus"`/`"click"`/`"worn"`/`"proc"` -> the source item's own
    /// base name already socketed there) -- straight off the dump, real
    /// ground truth for "what's already exalted onto this piece", not
    /// the proc-evidence-only inference `Ingest::exaltation_procs` still
    /// has to fall back on. See `EXALT_SOCKET_SUFFIXES`'s own doc for
    /// which socket numbers this covers (not `ornament`) and how
    /// confidently. A slot/socket combination with nothing socketed
    /// simply has no entry -- there is no empty placeholder to check
    /// against.
    pub exalted: HashMap<String, HashMap<String, String>>,
}

/// Parses a real `/outputfile inventory` dump (tab-separated: `Location`,
/// `Name`, `ID`, `Count`, `Slots` -- confirmed against a real dump, not
/// assumed). `equipped` only ever comes from the bare `SLOT_ORDER`
/// locations; `owned` sums every non-exalt-socket, non-empty row
/// regardless of location, since owning a copy in the bank counts same
/// as owning it in a bag.
///
/// A bag's own numbered contents (`General 1-Slot3`, the 3rd item in bag
/// "General 1") and *that* item's own augment sockets in turn
/// (`General 1-Slot3-Slot2`) look exactly like an equip-doll slot's own
/// exalt sockets do (`Back-Slot7`) -- both are a location with a
/// trailing `-Slot<N>` -- confirmed against a real dump this is a real,
/// nested nesting, not a parsing artifact. Distinguished here by walking
/// the file in order and remembering the two things that matter about
/// whatever base row was seen most recently: its own raw location string
/// (`last_base_location`) and, only when that row really was a real
/// equip-doll slot, which one (`last_equip_key`) -- a dump always lists
/// an exalt-socket row immediately after the row it belongs to, so this
/// single running pair is enough to tell a real `Back-Slot7` (whose
/// stripped base, `"Back"`, matches `last_base_location`, itself a real
/// equip slot) from a bag item's own nested one (whose stripped base,
/// `"General 1-Slot3"`, is not a real equip-doll location at all --
/// `last_equip_key` stays `None` for it, so it's correctly skipped, not
/// misattributed to whatever equip slot happened to be seen earlier in
/// the file).
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

        // why: a "-Slot<N>" row is a child of whatever base row came
        // immediately before it -- true for *every* augment/exalt
        // socket the dump carries, not just the 4 this module actually
        // models (`EXALT_SOCKET_SUFFIXES`). The real, reported bug this
        // guards against: the un-modelled ornament socket ("-Slot1" on
        // Head, "-Slot2" everywhere else, inconsistent -- see
        // `EXALT_SOCKET_SUFFIXES`'s own doc) used to fall all the way
        // through to the "new base row" branch below, overwriting
        // `last_base_location`/`last_equip_key` with itself -- which
        // then broke matching for every real exalt-socket row that
        // followed it in the same item's own block, since their own
        // stripped base no longer equalled the (corrupted)
        // `last_base_location`. Checked structurally (does stripping a
        // trailing "-Slot<digits>" land back on `last_base_location`),
        // not against the 4 modelled suffixes alone, so an unmodelled
        // socket type is silently skipped without corrupting state for
        // its real siblings either.
        if let Some(base) = strip_trailing_numbered_slot(location) {
            if base == last_base_location {
                if name != "Empty" {
                    if let (Some(equip_key), Some(socket_key)) = (last_equip_key, EXALT_SOCKET_SUFFIXES.iter().find(|(suffix, _)| location.ends_with(suffix)).map(|&(_, k)| k)) {
                        let source_name = name.strip_suffix(EXALTATION_SUFFIX).unwrap_or(name);
                        out.exalted.entry(equip_key.to_string()).or_default().insert(socket_key.to_string(), source_name.to_string());
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
            // More rows for this location than SLOT_ORDER expects for it --
            // an assumption this module makes turned out wrong for this
            // dump. Skip the overflow rather than silently misassigning it
            // to a slot it doesn't belong to.
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

    /// Real lines, unedited, from a real dump -- Back's own two socketed
    /// exaltations (focus + click), confirmed independently against
    /// `packs/items.json`'s own effect-type tags for both source items
    /// (see `EXALT_SOCKET_SUFFIXES`'s own doc for the full story).
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
        let back = parsed.exalted.get("BACK").expect("Back should have exalt sockets recorded");
        assert_eq!(back.get("focus"), Some(&"White Dragonscale Cloak".to_string()), "Slot7 -- confirmed focus-effect item");
        assert_eq!(back.get("click"), Some(&"Shield of the Immaculate".to_string()), "Slot8 -- confirmed click-effect item");
        assert_eq!(back.len(), 2, "the two Empty sockets (2 and 9) shouldn't produce entries");
    }

    /// Two copies of the same equip-doll slot (Ear1/Ear2), each with
    /// their own real exalt sockets -- the second Ear's own socketed
    /// exaltation must land on EAR2, not silently overwrite or merge
    /// into EAR1's.
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
        assert_eq!(parsed.exalted.get("EAR1").unwrap().get("click"), Some(&"Earring of Displacement".to_string()));
        assert!(!parsed.exalted.contains_key("EAR2"), "the second Ear had nothing real socketed");
    }

    /// A bag item's own nested exalt sockets (`General 1-Slot3-Slot2`,
    /// the item sitting in bag slot 3's *own* socket 2) look exactly
    /// like an equip-doll slot's exalt sockets do (a trailing
    /// `-Slot<N>`), confirmed against a real dump -- must not be
    /// misattributed to whatever equip-doll slot happened to be seen
    /// earlier in the file.
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
        let back = parsed.exalted.get("BACK").expect("Back's own real exalt socket");
        assert_eq!(back.len(), 1, "only Back's own real socket, not the bag item's nested one too");
        assert!(!parsed.exalted.contains_key("General 1"), "a bag is never an equip-doll slot");
    }

    /// A socketed exaltation source must never also inflate `owned` --
    /// it's consumed into the socket, not a spare copy sitting free.
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
        assert_eq!(parsed.owned.get("White Dragonscale Cloak (Exaltation)"), None);
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
