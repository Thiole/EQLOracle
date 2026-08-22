//! Reads EQ's own per-character UI config files out of the game's base
//! install folder (`AppConfig::base_dir`, same folder `/outputfile
//! inventory`/Achievements dumps sit in) -- confirmed against a real
//! install, two real kinds:
//!
//! - `<Character>_<Zone>_LO1.ini` -- **hotbutton contents**: what's
//!   actually assigned to each hotbutton slot (`[HotButtons]`'s own
//!   `Page1Button<N>=<code>,...` rows -- `H<n>` a built-in command,
//!   `G<n>` a reference to spell gem slot `n` (not the spell's own name
//!   -- which spell currently sits in that gem is server-tracked
//!   character state, never written to a local file at all, confirmed
//!   by its absence from every section here), `E<id>` an item, `J<n>`
//!   an AA -- plus a handful of small settings sections (`[Combat]`,
//!   `[Friends]`, ...).
//! - `UI_<Character>_<Zone>_LO1.ini` -- **window layout only**:
//!   position/size/visibility for every window (`[HotButtonWnd]`,
//!   `[CastSpellWnd]`, `[SpellBookWnd]`, ...), never *what's in* any of
//!   them.
//!
//! Both are plain Windows-style INI (`[Section]` headers, `key=value`
//! lines) -- but real-world messy: two of this player's own real files
//! were found with several thousand characters of unrelated pasted text
//! sitting *before* the first real `[Section]` header (apparently a
//! chat transcript that ended up saved into the wrong file). `parse_ini`
//! doesn't error on that -- it just can't attach a stray line to any
//! section until the first real header appears, so that prefix is
//! silently skipped rather than corrupting anything downstream. The
//! skipped count is still reported (`ParsedUiFileDto::skipped_garbage_
//! lines`) so a genuinely damaged file is visible, not silently clean.
//!
//! Read-only for now, on purpose: safely writing hotbutton assignments
//! back into one of these files without disturbing anything else in it
//! needs the encoding above nailed down with more certainty than one
//! read-through gives (`H0,@-1,0000000000000000,0,,` -- the trailing
//! fields past the type+id aren't understood yet), so this module only
//! ever reads.

use regex::Regex;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[derive(Debug, Clone, Serialize)]
pub struct UiFileInfoDto {
    pub file: String,
    pub character: String,
    pub zone: String,
    /// `"hotbuttons"` (`<Character>_<Zone>_LO1.ini`) or `"layout"`
    /// (`UI_<Character>_<Zone>_LO1.ini`) -- see this module's own doc
    /// for what each actually holds.
    pub kind: &'static str,
    /// A launcher/client-made backup copy (`..._Backup_1.ini` etc, a
    /// real, confirmed-seen naming convention) -- surfaced so the
    /// picker can label or de-prioritize these rather than mixing them
    /// in indistinguishably from the live file.
    pub is_backup: bool,
}

fn name_pattern() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^(?P<ui>UI_)?(?P<char>[^_]+)_(?P<zone>[^_]+)_LO1(?P<backup>_Backup_\d+)?\.ini$").unwrap())
}

/// Every real `<Character>_<Zone>_LO1.ini` / `UI_<Character>_<Zone>_
/// LO1.ini` sitting directly in `base_dir` -- not recursive, matching
/// where every real example was found. `base_dir` not existing or not
/// readable yields an empty list, same "nothing found yet" stance every
/// other dump-finder in this app takes.
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
            let kind = if caps.name("ui").is_some() { "layout" } else { "hotbuttons" };
            let is_backup = caps.name("backup").is_some();
            Some(UiFileInfoDto { file: name, character, zone, kind, is_backup })
        })
        .collect();
    out.sort_by(|a, b| (a.character.as_str(), a.zone.as_str(), a.kind, a.is_backup).cmp(&(b.character.as_str(), b.zone.as_str(), b.kind, b.is_backup)));
    out
}

#[derive(Debug, Clone, Serialize)]
pub struct UiSectionDto {
    pub name: String,
    /// In file order -- a repeated key within one section (not seen in
    /// any real file checked) would just keep its last value here,
    /// same as a real INI reader would.
    pub entries: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ParsedUiFileDto {
    pub sections: Vec<UiSectionDto>,
    /// See this module's own doc on the two real files found with a
    /// large pasted-text prefix before any real `[Section]` header --
    /// `0` for an ordinary, clean file.
    pub skipped_garbage_lines: usize,
}

/// `<base_dir>/<file>`, `file` trusted only as a bare filename (same
/// "never a path in its own right" stance `inventory::dump_path`
/// already takes, for the same reason: this always comes from a name
/// `list_ui_files` itself already found on disk, but staying defensive
/// costs nothing).
pub fn ui_file_path(base_dir: &Path, file: &str) -> std::io::Result<PathBuf> {
    let name_only = Path::new(file).file_name().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "not a bare filename"))?;
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
            sections.push(UiSectionDto { name: line[1..line.len() - 1].to_string(), entries: Vec::new() });
            continue;
        }
        let Some(current) = sections.last_mut() else {
            // Nothing to attach to yet -- either genuine pre-section
            // garbage (see this module's own doc), or a blank/odd line
            // right at the very top of an otherwise-clean file.
            skipped += 1;
            continue;
        };
        let Some((key, value)) = line.split_once('=') else {
            skipped += 1;
            continue;
        };
        current.entries.push((key.to_string(), value.to_string()));
    }

    Ok(ParsedUiFileDto { sections, skipped_garbage_lines: skipped })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn scratch_file(name: &str, text: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("eqlp-uifiles-{name}-{}.ini", std::process::id()));
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
        assert_eq!(parsed.sections[0].entries[1], ("Page1Button2".to_string(), "G4,@-1,0000000000000000,0,,".to_string()));
        assert_eq!(parsed.sections[1].entries, vec![("AttackOnAssist".to_string(), "1".to_string())]);
    }

    /// The exact real corruption found: a large pasted-text prefix with
    /// no `[Section]` header at all before the real content starts.
    /// Must not error, must not attach any of it to a section, and must
    /// report a nonzero skipped count.
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
        assert_eq!(parsed.sections[0].entries, vec![("UISkin".to_string(), "default_modern".to_string())]);
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
        let hotbuttons = files.iter().find(|f| f.file == "Manipulator_rivervale_LO1.ini").expect("hotbuttons file");
        assert_eq!(hotbuttons.character, "Manipulator");
        assert_eq!(hotbuttons.zone, "rivervale");
        assert_eq!(hotbuttons.kind, "hotbuttons");
        assert!(!hotbuttons.is_backup);
        let layout = files.iter().find(|f| f.file == "UI_Manipulator_rivervale_LO1.ini").expect("layout file");
        assert_eq!(layout.kind, "layout");
        let backup = files.iter().find(|f| f.file == "UI_Manipulator_qeynos_LO1_Backup_1.ini").expect("backup file");
        assert!(backup.is_backup);
        std::fs::remove_dir_all(&base).ok();
    }
}
