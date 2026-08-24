//! why: parses a real `<Character>-<Server>-Achievements.txt` dump, sitting
//! in the game's base folder like an `/outputfile inventory` dump -- not
//! triggered by this app, just read when already present.
//!
//! Format: `<status>\t<tabs-for-depth>text`, CRLF. `status` is I/C; a
//! bare category header has neither status nor tab.
//!
//! Used by `skyquests.rs`'s "Primary Class Unlocks" tab -- real
//! achievement-confirmed completion, not inferred from loot/inventory.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub struct Achievements {
    /// why: normalized line text -> complete; flat map, no tree needed yet
    complete: HashMap<String, bool>,
}

fn normalize(s: &str) -> String {
    s.trim().trim_end_matches('.').to_ascii_lowercase()
}

impl Achievements {
    /// why: None means text never appeared, distinct from Some(false)
    pub fn is_complete(&self, text: &str) -> Option<bool> {
        self.complete.get(&normalize(text)).copied()
    }
}

/// why: most recent dump already on disk, same stance as `inventory::find_existing_dump`
pub fn find_existing(base_dir: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(base_dir).ok()?;
    entries
        .filter_map(Result::ok)
        .filter(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| n.ends_with("-Achievements.txt"))
        })
        .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok())
        .map(|e| e.path())
}

pub fn parse(path: &Path) -> std::io::Result<Achievements> {
    let text = std::fs::read_to_string(path)?;
    let mut complete = HashMap::new();
    for line in text.lines() {
        let Some((status, rest)) = line.split_once('\t') else {
            continue; // why: bare category header, no status/tab
        };
        let body = rest.trim_start_matches('\t');
        if body.is_empty() {
            continue;
        }
        complete.insert(normalize(body), status == "C");
    }
    Ok(Achievements { complete })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_file(name: &str, text: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "eqlp-achievements-{name}-{}.txt",
            std::process::id()
        ));
        std::fs::write(&path, text).unwrap();
        path
    }

    /// why: real unedited lines, Bard block, all still incomplete
    const REAL_BARD_BLOCK: &str = "I\tPrimary Class Unlock - Bard\r\nI\t\tObtain Mask of Song.\r\nI\t\tObtain Mantle of the Songweaver.\r\nI\t\tObtain Ervaj's Flute of Flight.\r\n";

    #[test]
    fn real_incomplete_bard_lines_parse_as_incomplete() {
        let path = scratch_file("bard", REAL_BARD_BLOCK);
        let a = parse(&path).unwrap();
        assert_eq!(a.is_complete("Primary Class Unlock - Bard"), Some(false));
        // why: trailing period in file, none in lookup -- both normalize the same
        assert_eq!(a.is_complete("Obtain Mask of Song"), Some(false));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_complete_line_reports_true() {
        let path = scratch_file("complete", "C\tPrimary Class Unlock - Wizard\r\n");
        let a = parse(&path).unwrap();
        assert_eq!(
            a.is_complete("primary class unlock - wizard"),
            Some(true),
            "lookup should be case-insensitive too"
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn an_achievement_never_in_the_dump_reports_none_not_false() {
        let path = scratch_file("missing", "I\tPrimary Class Unlock - Bard\r\n");
        let a = parse(&path).unwrap();
        assert_eq!(a.is_complete("Primary Class Unlock - Wizard"), None);
        std::fs::remove_file(&path).ok();
    }

    /// why: bare category header must not panic or misread as real text
    #[test]
    fn a_bare_category_header_line_is_skipped_without_panicking() {
        let path = scratch_file(
            "header",
            "Untapped Potential: Races\r\nI\tRace Unlock - Barbarian\r\n",
        );
        let a = parse(&path).unwrap();
        assert_eq!(a.is_complete("Untapped Potential: Races"), None);
        assert_eq!(a.is_complete("Race Unlock - Barbarian"), Some(false));
        std::fs::remove_file(&path).ok();
    }
}
