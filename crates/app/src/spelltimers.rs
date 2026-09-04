//! why: the game's own spell data for what the wiki gets wrong or lacks
//! -- the install's `spells_us.txt` (173 `^` columns). Read here: cast
//! time (col 8, ms), recast (col 10, ms) and the shared reuse timer id
//! (col 55). Verified 2026-09-03 on the real file: Lifebite cast 1750
//! recast 1500 where the wiki page has neither; Spike/Spear of
//! Disease/Spear of Pain share timer 22, the rains 3, Conflagration 0.
//! input: the install folder; output: per spell name

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

const CAST_COL: usize = 8;
const RECAST_COL: usize = 10;
const TIMER_ID_COL: usize = 55;
/// why: 16 per-class level requirements, 255 = that class cannot cast it
/// (L8/G6). Verified on the real file: Conflagration WIZ 43, Lifedraw
/// NEC 12 / SHD 15, Improved Invisibility WIZ 55 and ENC 50.
const CLASS_LEVEL_COL: usize = 36;
/// why: the file's own class order, classic-EQ, not this app's alphabetical
const FILE_CLASSES: [&str; 16] = [
    "Warrior",
    "Cleric",
    "Paladin",
    "Ranger",
    "Shadow Knight",
    "Druid",
    "Monk",
    "Bard",
    "Rogue",
    "Shaman",
    "Necromancer",
    "Wizard",
    "Magician",
    "Enchanter",
    "Beastlord",
    "Berserker",
];
/// why: the file's "no" value for a class column
const NOT_CASTABLE: u8 = 255;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpellFileEntry {
    pub cast_ms: u32,
    pub recast_ms: u32,
    /// why: 0 means no shared timer
    pub timer: u32,
    /// why: level required per class, `FILE_CLASSES` order, 255 = never
    pub levels: [u8; 16],
}

pub type SpellFile = Arc<HashMap<String, SpellFileEntry>>;

fn cache() -> &'static Mutex<HashMap<PathBuf, SpellFile>> {
    static C: OnceLock<Mutex<HashMap<PathBuf, SpellFile>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

fn parse(text: &str) -> HashMap<String, SpellFileEntry> {
    let mut out = HashMap::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.split('^').collect();
        if fields.len() <= TIMER_ID_COL {
            continue;
        }
        let name = fields[1];
        let num = |i: usize| fields[i].parse::<u32>().unwrap_or(0);
        let mut levels = [NOT_CASTABLE; 16];
        for (i, slot) in levels.iter_mut().enumerate() {
            if let Some(f) = fields.get(CLASS_LEVEL_COL + i) {
                *slot = f.parse::<u8>().unwrap_or(NOT_CASTABLE);
            }
        }
        let entry = SpellFileEntry {
            cast_ms: num(CAST_COL),
            recast_ms: num(RECAST_COL),
            timer: num(TIMER_ID_COL),
            levels,
        };
        // why: the file repeats some names, the later row often a
        // non-castable stub (every class 255) -- keep the row that
        // actually states levels rather than whichever came first
        match out.entry(name.to_ascii_lowercase()) {
            std::collections::hash_map::Entry::Vacant(v) => {
                v.insert(entry);
            }
            std::collections::hash_map::Entry::Occupied(mut o) => {
                let have = o.get().levels.iter().any(|l| *l != NOT_CASTABLE);
                if !have && levels.iter().any(|l| *l != NOT_CASTABLE) {
                    o.insert(entry);
                }
            }
        }
    }
    out
}

/// why: read once per install folder; a missing file reads as an empty
/// map, never an error -- every caller falls back to the wiki pack
pub fn spell_file(base_dir: &Path) -> SpellFile {
    if let Some(t) = cache().lock().ok().and_then(|c| c.get(base_dir).cloned()) {
        return t;
    }
    let text = std::fs::read(base_dir.join("spells_us.txt"))
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default();
    let t: SpellFile = Arc::new(parse(&text));
    if let Ok(mut c) = cache().lock() {
        c.insert(base_dir.to_path_buf(), t.clone());
    }
    t
}

pub fn entry_of(file: &SpellFile, name: &str) -> Option<SpellFileEntry> {
    file.get(&name.to_ascii_lowercase()).copied()
}

/// why: L8 -- the game's own per-class level requirements for one spell,
/// this server's numbers rather than a wiki scrape. Empty when the spell
/// is unknown or no class can cast it.
pub fn class_levels(file: &SpellFile, name: &str) -> Vec<(String, u8)> {
    let Some(e) = entry_of(file, name) else {
        return Vec::new();
    };
    e.levels
        .iter()
        .enumerate()
        .filter(|(_, l)| **l != NOT_CASTABLE && **l > 0)
        .map(|(i, l)| (FILE_CLASSES[i].to_string(), *l))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: u32, name: &str, cast: u32, recast: u32, timer: u32) -> String {
        let mut f = vec!["0".to_string(); 60];
        f[0] = id.to_string();
        f[1] = name.to_string();
        f[CAST_COL] = cast.to_string();
        f[RECAST_COL] = recast.to_string();
        f[TIMER_ID_COL] = timer.to_string();
        f.join("^")
    }

    /// why: L8's data source -- the install's own per-class levels, and
    /// the duplicate-name stub row must not win over the real one
    #[test]
    fn class_levels_read_from_the_files_own_columns() {
        let mut f = vec!["0".to_string(); 60];
        f[1] = "Improved Invisibility".to_string();
        for i in 0..16 {
            f[CLASS_LEVEL_COL + i] = "255".to_string();
        }
        f[CLASS_LEVEL_COL + 11] = "55".to_string(); // Wizard
        f[CLASS_LEVEL_COL + 13] = "50".to_string(); // Enchanter
        let real = f.join("^");
        let mut stub = vec!["0".to_string(); 60];
        stub[1] = "Improved Invisibility".to_string();
        for i in 0..16 {
            stub[CLASS_LEVEL_COL + i] = "255".to_string();
        }
        let file: SpellFile = Arc::new(parse(&format!("{}\n{}", stub.join("^"), real)));
        let mut got = class_levels(&file, "improved invisibility");
        got.sort();
        assert_eq!(
            got,
            vec![
                ("Enchanter".to_string(), 50u8),
                ("Wizard".to_string(), 55u8)
            ],
            "the stub row must not shadow the row that states levels"
        );
    }

    /// why: the real columns, the real groups Spencer named
    #[test]
    fn cast_recast_and_timer_read_from_their_columns() {
        let text = [
            row(1, "Spike of Disease", 500, 45000, 22),
            row(2, "Spear of Pain", 500, 45000, 22),
            row(3, "Lifebite", 1750, 1500, 0),
            row(4, "Conflagration", 5000, 1500, 0),
        ]
        .join("\n");
        let f: SpellFile = Arc::new(parse(&text));
        assert_eq!(entry_of(&f, "Spear of Pain").map(|e| e.timer), Some(22));
        assert_eq!(
            entry_of(&f, "lifebite").map(|e| (e.cast_ms, e.recast_ms)),
            Some((1750, 1500))
        );
        assert_eq!(entry_of(&f, "Conflagration").map(|e| e.timer), Some(0));
        assert_eq!(entry_of(&f, "Nope"), None);
    }
}
