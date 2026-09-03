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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpellFileEntry {
    pub cast_ms: u32,
    pub recast_ms: u32,
    /// why: 0 means no shared timer
    pub timer: u32,
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
        out.entry(name.to_ascii_lowercase())
            .or_insert(SpellFileEntry {
                cast_ms: num(CAST_COL),
                recast_ms: num(RECAST_COL),
                timer: num(TIMER_ID_COL),
            });
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
