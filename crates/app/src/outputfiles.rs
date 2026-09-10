//! why: the game's `/outputfile` dumps beside Logs -- which exist, the
//! command that writes each, and the spellbook rows nothing read before.

use eqlp_source::Millis;
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// why: (kind, filename suffix, in-game command) -- the three the app reads
pub const KINDS: &[(&str, &str, &str)] = &[
    ("inventory", "-Inventory.txt", "/outputfile inventory"),
    (
        "achievements",
        "-Achievements.txt",
        "/outputfile achievements",
    ),
    ("spellbook", "-Spellbook.txt", "/outputfile spellbook"),
];

#[derive(Debug, Clone, Serialize)]
pub struct OutputfileDto {
    pub kind: String,
    pub command: String,
    pub file: Option<String>,
    pub modified_ms: Option<Millis>,
}

fn modified_ms(path: &Path) -> Option<Millis> {
    let t = std::fs::metadata(path).ok()?.modified().ok()?;
    let d = t.duration_since(std::time::UNIX_EPOCH).ok()?;
    Millis::try_from(d.as_millis()).ok()
}

fn newest(base_dir: &Path, suffix: &str) -> Option<PathBuf> {
    std::fs::read_dir(base_dir)
        .ok()?
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_str().is_some_and(|n| n.ends_with(suffix)))
        .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok())
        .map(|e| e.path())
}

/// why: newest dump per kind -- the Import menu's status rows
pub fn list(base_dir: &Path) -> Vec<OutputfileDto> {
    KINDS
        .iter()
        .map(|(kind, suffix, command)| {
            let path = newest(base_dir, suffix);
            OutputfileDto {
                kind: kind.to_string(),
                command: command.to_string(),
                file: path
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().into_owned()),
                modified_ms: path.as_deref().and_then(modified_ms),
            }
        })
        .collect()
}

/// why: every class file of one character (`<Char>_<server>-<CLASS>-Spellbook.txt`,
/// rows `level<TAB>name`); the file's own mtime stands in for first seen
pub fn spellbook_spells(base_dir: &Path, character: Option<&str>) -> Vec<(String, Millis)> {
    let prefix = character.map(|c| format!("{c}_"));
    let Ok(entries) = std::fs::read_dir(base_dir) else {
        return Vec::new();
    };
    let mut seen: HashMap<String, Millis> = HashMap::new();
    for e in entries.filter_map(Result::ok) {
        let name = e.file_name().to_string_lossy().into_owned();
        let mine = prefix.as_ref().is_none_or(|p| name.starts_with(p.as_str()));
        if !name.ends_with("-Spellbook.txt") || !mine {
            continue;
        }
        let ts = modified_ms(&e.path()).unwrap_or(0);
        let Ok(text) = std::fs::read_to_string(e.path()) else {
            continue;
        };
        for line in text.lines() {
            let Some((level, spell)) = line.split_once('\t') else {
                continue;
            };
            let spell = spell.trim();
            if level.trim().parse::<u16>().is_err() || spell.is_empty() {
                continue;
            }
            seen.entry(spell.to_string()).or_insert(ts);
        }
    }
    seen.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_the_newest_per_kind_and_reads_one_characters_spellbook_rows() {
        let d = std::env::temp_dir().join(format!("eqlp-outputfiles-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join("Manipulator_rivervale-Inventory.txt"),
            "Location\tName\n",
        )
        .unwrap();
        std::fs::write(
            d.join("Manipulator_rivervale-WIZ-Spellbook.txt"),
            "1\tBlast of Cold\r\n4\tFrost Bolt\r\nno tab here\r\n",
        )
        .unwrap();
        std::fs::write(
            d.join("Manipulator_rivervale-ENC-Spellbook.txt"),
            "1\tBlast of Cold\r\n2\tMesmerize\r\n",
        )
        .unwrap();
        std::fs::write(d.join("Alt_rivervale-BRD-Spellbook.txt"), "1\tChant\r\n").unwrap();

        let listed = list(&d);
        assert_eq!(listed.len(), 3);
        assert_eq!(
            listed[0].file.as_deref(),
            Some("Manipulator_rivervale-Inventory.txt")
        );
        assert!(listed[0].modified_ms.is_some());
        assert_eq!(listed[1].file, None, "no achievements dump written");
        assert_eq!(listed[1].command, "/outputfile achievements");

        let mut spells: Vec<String> = spellbook_spells(&d, Some("Manipulator"))
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        spells.sort();
        assert_eq!(
            spells,
            ["Blast of Cold", "Frost Bolt", "Mesmerize"],
            "both class files, deduped, the alt's file left out"
        );
        let _ = std::fs::remove_dir_all(&d);
    }
}
