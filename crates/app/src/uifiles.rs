//! why: reads EQ's per-character UI config files from `AppConfig::base_dir`
//!
//! Two kinds: `<Character>_<Zone>_LO1.ini` (hotbutton contents -- which
//! gem/item/AA/command sits in each slot) and `UI_<Character>_<Zone>_
//! LO1.ini` (window layout only, never contents). Plain INI, but
//! real-world messy: real files found with pasted-text garbage before
//! the first `[Section]` -- skipped, not corrupting, count still reported.
//!
//! Read-only on purpose: writing back needs the trailing-field encoding
//! nailed down further than one read-through gives.

use regex::Regex;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[derive(Debug, Clone, Serialize)]
pub struct UiFileInfoDto {
    pub file: String,
    pub character: String,
    pub zone: String,
    /// why: "hotbuttons" or "layout" -- see module doc for what each holds
    pub kind: &'static str,
    /// why: launcher-made backup copy, surfaced so the picker can label it
    pub is_backup: bool,
}

fn name_pattern() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"^(?P<ui>UI_)?(?P<char>[^_]+)_(?P<zone>[^_]+)_LO1(?P<backup>_Backup_\d+)?\.ini$",
        )
        .unwrap()
    })
}

/// why: every real UI file directly in `base_dir`, not recursive;
/// missing/unreadable dir yields empty list
pub fn list_ui_files(base_dir: &Path) -> Vec<UiFileInfoDto> {
    let Ok(entries) = std::fs::read_dir(base_dir) else {
        return Vec::new();
    };
    let re = name_pattern();
    let mut out: Vec<UiFileInfoDto> = entries
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let caps = re.captures(&name)?;
            let character = caps["char"].to_string();
            let zone = caps["zone"].to_string();
            let kind = if caps.name("ui").is_some() {
                "layout"
            } else {
                "hotbuttons"
            };
            let is_backup = caps.name("backup").is_some();
            Some(UiFileInfoDto {
                file: name,
                character,
                zone,
                kind,
                is_backup,
            })
        })
        .collect();
    out.sort_by(|a, b| {
        (a.character.as_str(), a.zone.as_str(), a.kind, a.is_backup).cmp(&(
            b.character.as_str(),
            b.zone.as_str(),
            b.kind,
            b.is_backup,
        ))
    });
    out
}

#[derive(Debug, Clone, Serialize)]
pub struct UiSectionDto {
    pub name: String,
    /// why: file order; a repeated key would keep its last value, like a real INI reader
    pub entries: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ParsedUiFileDto {
    pub sections: Vec<UiSectionDto>,
    /// why: nonzero only for real files with a pasted-text prefix garbage
    pub skipped_garbage_lines: usize,
}

/// why: `file` trusted only as a bare filename, same stance as `inventory::dump_path`
pub fn ui_file_path(base_dir: &Path, file: &str) -> std::io::Result<PathBuf> {
    let name_only = Path::new(file).file_name().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "not a bare filename")
    })?;
    Ok(base_dir.join(name_only))
}

pub fn parse_ini(path: &Path) -> std::io::Result<ParsedUiFileDto> {
    let text = std::fs::read_to_string(path)?;
    let mut sections: Vec<UiSectionDto> = Vec::new();
    let mut skipped = 0usize;

    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') && line.len() > 2 {
            sections.push(UiSectionDto {
                name: line[1..line.len() - 1].to_string(),
                entries: Vec::new(),
            });
            continue;
        }
        let Some(current) = sections.last_mut() else {
            // why: nothing to attach to yet -- pre-section garbage or a stray top line
            skipped += 1;
            continue;
        };
        let Some((key, value)) = line.split_once('=') else {
            skipped += 1;
            continue;
        };
        current.entries.push((key.to_string(), value.to_string()));
    }

    Ok(ParsedUiFileDto {
        sections,
        skipped_garbage_lines: skipped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn scratch_file(name: &str, text: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("eqlp-uifiles-{name}-{}.ini", std::process::id()));
        std::fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn a_clean_file_parses_every_section_and_entry() {
        let path = scratch_file(
            "clean",
            "[HotButtons]\nPage1Button1=H1,@-1,0000000000000000,0,,\nPage1Button2=G4,@-1,0000000000000000,0,,\n[Combat]\nAttackOnAssist=1\n",
        );
        let parsed = parse_ini(&path).unwrap();
        assert_eq!(parsed.skipped_garbage_lines, 0);
        assert_eq!(parsed.sections.len(), 2);
        assert_eq!(parsed.sections[0].name, "HotButtons");
        assert_eq!(
            parsed.sections[0].entries[1],
            (
                "Page1Button2".to_string(),
                "G4,@-1,0000000000000000,0,,".to_string()
            )
        );
        assert_eq!(
            parsed.sections[1].entries,
            vec![("AttackOnAssist".to_string(), "1".to_string())]
        );
    }

    /// why: real corruption case -- pasted-text prefix, must not error or misattach
    #[test]
    fn a_pasted_text_prefix_before_the_first_section_is_skipped_not_misparsed() {
        let path = scratch_file(
            "garbage",
            "I need help configuring this into something very cohesive.\nthe theme I want is bars on bottom for hotkeys\n[Main]\nUISkin=default_modern\n",
        );
        let parsed = parse_ini(&path).unwrap();
        assert_eq!(parsed.skipped_garbage_lines, 2);
        assert_eq!(parsed.sections.len(), 1);
        assert_eq!(parsed.sections[0].name, "Main");
        assert_eq!(
            parsed.sections[0].entries,
            vec![("UISkin".to_string(), "default_modern".to_string())]
        );
    }

    #[test]
    fn real_filenames_resolve_to_the_right_character_zone_and_kind() {
        let base = std::env::temp_dir().join(format!("eqlp-uifiles-list-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        for name in [
            "Manipulator_rivervale_LO1.ini",
            "UI_Manipulator_rivervale_LO1.ini",
            "UI_Manipulator_qeynos_LO1_Backup_1.ini",
            "not_a_real_ui_file.txt",
        ] {
            std::fs::write(base.join(name), "").unwrap();
        }
        let files = list_ui_files(&base);
        assert_eq!(files.len(), 3, "the non-matching file must not appear");
        let hotbuttons = files
            .iter()
            .find(|f| f.file == "Manipulator_rivervale_LO1.ini")
            .expect("hotbuttons file");
        assert_eq!(hotbuttons.character, "Manipulator");
        assert_eq!(hotbuttons.zone, "rivervale");
        assert_eq!(hotbuttons.kind, "hotbuttons");
        assert!(!hotbuttons.is_backup);
        let layout = files
            .iter()
            .find(|f| f.file == "UI_Manipulator_rivervale_LO1.ini")
            .expect("layout file");
        assert_eq!(layout.kind, "layout");
        let backup = files
            .iter()
            .find(|f| f.file == "UI_Manipulator_qeynos_LO1_Backup_1.ini")
            .expect("backup file");
        assert!(backup.is_backup);
        std::fs::remove_dir_all(&base).ok();
    }
}
