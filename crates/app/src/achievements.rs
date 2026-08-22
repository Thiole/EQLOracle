//! Parses a real `<Character>-<Server>-Achievements.txt` dump, sitting
//! directly in the game's base install folder the same way an
//! `/outputfile inventory` dump does (confirmed against a real file,
//! `Manipulator_rivervale-Achievements.txt`) -- not something this app
//! triggers itself, just read whenever one's already there.
//!
//! Format, confirmed directly against that real file: one line per
//! achievement or requirement, `<status>\t<tabs-for-depth>text`, CRLF
//! line endings (`str::lines()` already strips both `\n` and `\r\n`, no
//! manual handling needed). `status` is `I` (incomplete) or `C`
//! (complete); a bare category header line (`Untapped Potential: Races`)
//! carries neither a status letter nor a leading tab at all -- harmless
//! here since nothing queries those, but worth knowing if this ever
//! grows a real tree view.
//!
//! First real, verified use: `skyquests.rs`'s "Primary Class Unlocks"
//! tab reads two exact line shapes out of this same file --
//! `"Primary Class Unlock - <Class>"` (the class's own overall unlock
//! status) and `"Obtain <Reward>."` (one per quest, matching `sky_quests
//! .json`'s own `reward` field verbatim, confirmed: "Obtain Mask of
//! Song." sits directly under "Primary Class Unlock - Bard", and
//! `sky_quests.json`'s own Bard "Test of Tone" quest's reward is "Mask
//! of Song") -- real achievement-confirmed completion, not inferred from
//! loot/inventory the way an unscraped turn-in still has to be.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub struct Achievements {
    /// Normalized (trimmed, trailing "." dropped, lowercased) line text
    /// -> complete. A flat map, not a tree: every real lookup so far is
    /// "is this exact line complete", never "what are this line's own
    /// children" -- see this module's own doc if that ever changes.
    complete: HashMap<String, bool>,
}

fn normalize(s: &str) -> String {
    s.trim().trim_end_matches('.').to_ascii_lowercase()
}

impl Achievements {
    /// `None` if `text` never appears as a real line in this dump at all
    /// (a stale scrape, a wiki-name mismatch, or this achievement not
    /// existing under this exact wording) -- distinct from `Some(false)`,
    /// a real line that's genuinely still incomplete.
    pub fn is_complete(&self, text: &str) -> Option<bool> {
        self.complete.get(&normalize(text)).copied()
    }
}

/// The most recently written Achievements dump already sitting in
/// `base_dir`, if any -- same "offer what's already on disk, don't wait
/// on a fresh write this session" stance `inventory::find_existing_dump`
/// takes, for the same reason.
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
            continue; // a bare category header -- see this module's own doc
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

    /// Real lines, unedited, from `Manipulator_rivervale-Achievements.txt`
    /// (Bard's own block) -- Bard is still fully incomplete in this
    /// character's real dump, every sub-requirement `I`.
    const REAL_BARD_BLOCK: &str = "I\tPrimary Class Unlock - Bard\r\nI\t\tObtain Mask of Song.\r\nI\t\tObtain Mantle of the Songweaver.\r\nI\t\tObtain Ervaj's Flute of Flight.\r\n";

    #[test]
    fn real_incomplete_bard_lines_parse_as_incomplete() {
        let path = scratch_file("bard", REAL_BARD_BLOCK);
        let a = parse(&path).unwrap();
        assert_eq!(a.is_complete("Primary Class Unlock - Bard"), Some(false));
        // Trailing period in the real file, none in the lookup -- both
        // sides normalize the same way.
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

    /// A bare category header line (no status letter, no tab at all)
    /// must not panic or get misread as real achievement text.
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
