//! why: which spells share a reuse timer -- the game's own answer, not
//! a guess from shape. The install's `spells_us.txt` carries a timer
//! id per spell (column 55, confirmed against Spencer's own list:
//! Spike/Spear of Disease/Spear of Pain share 22, the rains share 3,
//! Conflagration 0). Spells with the same nonzero id lock together.
//! input: the install folder; output: timer id by spell name

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

/// why: `^`-separated field index of the timer id -- verified on the
/// real file, see the module doc
const TIMER_ID_COL: usize = 55;

type Timers = Arc<HashMap<String, u32>>;

fn cache() -> &'static Mutex<HashMap<PathBuf, Timers>> {
    static C: OnceLock<Mutex<HashMap<PathBuf, Timers>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

fn parse(text: &str) -> HashMap<String, u32> {
    let mut out = HashMap::new();
    for line in text.lines() {
        let mut fields = line.split('^');
        let Some(_id) = fields.next() else { continue };
        let Some(name) = fields.next() else { continue };
        let Some(timer) = fields.nth(TIMER_ID_COL - 2) else {
            continue;
        };
        let Ok(timer) = timer.parse::<u32>() else {
            continue;
        };
        if timer == 0 {
            continue;
        }
        out.entry(name.to_ascii_lowercase()).or_insert(timer);
    }
    out
}

/// why: read once per install folder; a missing file reads as "no
/// timer known for anything", never an error
pub fn timers(base_dir: &Path) -> Timers {
    if let Some(t) = cache().lock().ok().and_then(|c| c.get(base_dir).cloned()) {
        return t;
    }
    let text = std::fs::read(base_dir.join("spells_us.txt"))
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default();
    let t: Timers = Arc::new(parse(&text));
    if let Ok(mut c) = cache().lock() {
        c.insert(base_dir.to_path_buf(), t.clone());
    }
    t
}

/// why: the shared timer id, if the spell has one
pub fn timer_of(timers: &Timers, name: &str) -> Option<u32> {
    timers.get(&name.to_ascii_lowercase()).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: u32, name: &str, timer: u32) -> String {
        let mut f = vec!["0".to_string(); 60];
        f[0] = id.to_string();
        f[1] = name.to_string();
        f[TIMER_ID_COL] = timer.to_string();
        f.join("^")
    }

    /// why: the real column, the real groups Spencer named
    #[test]
    fn spells_with_the_same_nonzero_id_share_and_zero_means_none() {
        let text = [
            row(1, "Spike of Disease", 22),
            row(2, "Spear of Disease", 22),
            row(3, "Spear of Pain", 22),
            row(4, "Lava Storm", 3),
            row(5, "Frost Storm", 3),
            row(6, "Conflagration", 0),
        ]
        .join("\n");
        let t: Timers = Arc::new(parse(&text));
        assert_eq!(timer_of(&t, "Spear of Pain"), Some(22));
        assert_eq!(timer_of(&t, "spike of disease"), Some(22));
        assert_eq!(timer_of(&t, "Frost Storm"), Some(3));
        assert_eq!(timer_of(&t, "Conflagration"), None);
    }
}
