//! why: reads/writes a character's real `[SpellLoadouts]` section -- the
//! game's own saved spellbook loadouts, stored in the same non-`UI_`-
//! prefixed `.ini` files `uifiles.rs` already discovers (see its own doc
//! for the file-pairing convention this reuses wholesale).
//!
//! A loadout's slots hold numeric spell ids, not names -- resolved
//! through the install folder's own `spells_us.txt` (a legacy classic-EQ
//! id/name table). Confirmed against a real character's 21 real
//! loadouts: every id resolves through `spells_us.txt` (0 misses), and
//! ~96% of those names then case-insensitively match `packs/spells.json`
//! for a Game Data deep link -- the rest link up to nothing, which is
//! exactly what "link up what you can" means: best-effort, not invented.

use crate::spelldata;
use crate::uifiles;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// why: the real, fixed per-loadout slot count this client itself uses
/// -- confirmed against every real loadout file found (21 real named
/// loadouts across two characters, every one tops out at slot14, never past it)
pub const MAX_SLOTS: u32 = 14;
/// why: the real, fixed loadout-slot count the client reserves --
/// confirmed real: SpellLoadout1..SpellLoadout60, unused ones a single
/// bare ".inuse=0" line each (line count matched exactly: 21 in-use *
/// 16 lines + 39 unused * 1 line = the section's real total)
pub const MAX_LOADOUTS: u32 = 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadoutSlotDto {
    pub slot: u32,
    /// why: -1 is the real "empty" sentinel this game's own files use
    pub spell_id: i64,
    /// why: resolved via spells_us.txt; None for spell_id == -1 or an
    /// id spells_us.txt itself doesn't know (not observed real, kept honest anyway)
    pub name: Option<String>,
    /// why: packs/spells.json's own id, best-effort case-insensitive
    /// name match -- lets the frontend deep-link to Game Data. None is
    /// common (real: ~4% of a real spellbook) and not an error.
    pub catalog_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpellLoadoutDto {
    pub index: u32,
    pub in_use: bool,
    pub name: Option<String>,
    /// why: always MAX_SLOTS long when in_use, empty when not
    pub slots: Vec<LoadoutSlotDto>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SpellbookFileDto {
    pub file: String,
    pub loadouts: Vec<SpellLoadoutDto>,
}

/// why: id<->name, read fresh from the install folder each call -- this
/// file changes rarely and only while eqlp isn't the one holding it
/// open, no reason to risk a stale cache
struct SpellIdIndex {
    by_id: HashMap<i64, String>,
    /// why: lowercased key, first id wins on a real duplicate name --
    /// same ambiguous-name stance spelltext.rs takes, just permissive
    /// here since a slot is a single concrete pick, not a classification
    by_name: HashMap<String, i64>,
}

fn build_spell_id_index(base_dir: &Path) -> SpellIdIndex {
    let mut idx = SpellIdIndex {
        by_id: HashMap::new(),
        by_name: HashMap::new(),
    };
    let Ok(bytes) = std::fs::read(base_dir.join("spells_us.txt")) else {
        return idx;
    };
    let text = String::from_utf8_lossy(&bytes);
    for line in text.lines() {
        let mut parts = line.splitn(3, '^');
        let Some(id_str) = parts.next() else { continue };
        let Some(name) = parts.next() else { continue };
        let Ok(id) = id_str.parse::<i64>() else {
            continue;
        };
        idx.by_name.entry(name.to_ascii_lowercase()).or_insert(id);
        idx.by_id.insert(id, name.to_string());
    }
    idx
}

/// why: packs/spells.json is baked-in and 'static -- this reverse index
/// never changes across calls, unlike spell_us.txt's own live file
fn catalog_index() -> &'static HashMap<String, &'static str> {
    static IDX: OnceLock<HashMap<String, &'static str>> = OnceLock::new();
    IDX.get_or_init(|| {
        spelldata::spells()
            .iter()
            .map(|s| (s.name.to_ascii_lowercase(), s.id.as_str()))
            .collect()
    })
}

#[derive(Default)]
struct RawLoadout {
    in_use: bool,
    name: Option<String>,
    slots: HashMap<u32, i64>,
}

fn loadouts_from_ini(
    parsed: &uifiles::ParsedUiFileDto,
    ids: &SpellIdIndex,
) -> Vec<SpellLoadoutDto> {
    let mut raw: HashMap<u32, RawLoadout> = HashMap::new();
    if let Some(section) = parsed.sections.iter().find(|s| s.name == "SpellLoadouts") {
        for (key, value) in &section.entries {
            let Some(rest) = key.strip_prefix("SpellLoadout") else {
                continue;
            };
            let Some((idx_str, field)) = rest.split_once('.') else {
                continue;
            };
            let Ok(idx) = idx_str.parse::<u32>() else {
                continue;
            };
            let entry = raw.entry(idx).or_default();
            if field == "inuse" {
                entry.in_use = value == "1";
            } else if field == "name" {
                entry.name = Some(value.clone());
            } else if let Some(slot_str) = field.strip_prefix("slot") {
                if let (Ok(slot_n), Ok(id)) = (slot_str.parse::<u32>(), value.parse::<i64>()) {
                    entry.slots.insert(slot_n, id);
                }
            }
        }
    }

    (1..=MAX_LOADOUTS)
        .map(|idx| {
            let r = raw.remove(&idx).unwrap_or_default();
            let slots = if r.in_use {
                (1..=MAX_SLOTS)
                    .map(|slot| {
                        let spell_id = r.slots.get(&slot).copied().unwrap_or(-1);
                        let name = (spell_id != -1)
                            .then(|| ids.by_id.get(&spell_id).cloned())
                            .flatten();
                        let catalog_id = name
                            .as_deref()
                            .and_then(|n| catalog_index().get(&n.to_ascii_lowercase()))
                            .map(|s| s.to_string());
                        LoadoutSlotDto {
                            slot,
                            spell_id,
                            name,
                            catalog_id,
                        }
                    })
                    .collect()
            } else {
                Vec::new()
            };
            SpellLoadoutDto {
                index: idx,
                in_use: r.in_use,
                name: r.name,
                slots,
            }
        })
        .collect()
}

/// why: `file` trusted only as a bare filename, same stance as `uifiles::ui_file_path`
pub fn load_spellbook(base_dir: &Path, file: &str) -> Result<SpellbookFileDto, String> {
    let path = uifiles::ui_file_path(base_dir, file).map_err(|e| e.to_string())?;
    let parsed = uifiles::parse_ini(&path).map_err(|e| e.to_string())?;
    let ids = build_spell_id_index(base_dir);
    Ok(SpellbookFileDto {
        file: file.to_string(),
        loadouts: loadouts_from_ini(&parsed, &ids),
    })
}

/// why: resolves a spell name to its real numeric id, for the frontend
/// placing a fresh catalog spell into an empty/edited slot -- None means
/// spells_us.txt has no entry under that exact name (real, confirmed:
/// ~7% of the full catalog), not that the name is wrong
pub fn resolve_spell_id(base_dir: &Path, name: &str) -> Option<i64> {
    build_spell_id_index(base_dir)
        .by_name
        .get(&name.to_ascii_lowercase())
        .copied()
}

/// why: shared by save_spellbook and save_spellbook_as -- always writes
/// exactly MAX_LOADOUTS entries, rejects a mismatched shape rather than
/// silently writing a file the real client can't read.
fn validate_loadouts_shape(loadouts: &[SpellLoadoutDto]) -> Result<(), String> {
    if loadouts.len() != MAX_LOADOUTS as usize {
        return Err(format!(
            "expected exactly {MAX_LOADOUTS} loadout entries, got {}",
            loadouts.len()
        ));
    }
    for (i, lo) in loadouts.iter().enumerate() {
        let want_idx = i as u32 + 1;
        if lo.index != want_idx {
            return Err(format!(
                "loadout entries must be indexed 1..={MAX_LOADOUTS} in order, found index {} at position {i}",
                lo.index
            ));
        }
        if lo.in_use {
            if lo.name.as_deref().unwrap_or("").is_empty() {
                return Err(format!("loadout {want_idx} is in use but has no name"));
            }
            if lo.slots.len() != MAX_SLOTS as usize
                || !(1..=MAX_SLOTS).all(|s| lo.slots.iter().any(|slot| slot.slot == s))
            {
                return Err(format!(
                    "loadout {want_idx} must have exactly slots 1..={MAX_SLOTS}"
                ));
            }
        }
    }
    Ok(())
}

/// why: shared by save_spellbook and save_spellbook_as -- rebuilds the
/// [SpellLoadouts] block from `loadouts`, byte-for-byte preserving every
/// other section of `original` -- HotButtons/Combat/etc. use a much
/// denser trailing-field encoding this app has never needed to fully
/// nail down (see uifiles.rs's own doc), so this never touches them.
fn splice_spell_loadouts_block(original: &str, loadouts: &[SpellLoadoutDto]) -> String {
    let lines: Vec<&str> = original.lines().collect();
    let section_start = lines.iter().position(|l| l.trim() == "[SpellLoadouts]");
    let (before, after): (Vec<&str>, Vec<&str>) = match section_start {
        Some(i) => {
            let rel_end = lines[i + 1..]
                .iter()
                .position(|l| l.trim_start().starts_with('['));
            let end = rel_end.map(|o| i + 1 + o).unwrap_or(lines.len());
            (lines[..i].to_vec(), lines[end..].to_vec())
        }
        // why: no section yet -- appended at the end, nothing to preserve after it
        None => (lines.clone(), Vec::new()),
    };

    let mut body = vec!["[SpellLoadouts]".to_string()];
    for lo in loadouts {
        body.push(format!(
            "SpellLoadout{}.inuse={}",
            lo.index,
            if lo.in_use { 1 } else { 0 }
        ));
        if lo.in_use {
            body.push(format!(
                "SpellLoadout{}.name={}",
                lo.index,
                lo.name.as_deref().unwrap_or("")
            ));
            for slot in &lo.slots {
                body.push(format!(
                    "SpellLoadout{}.slot{}={}",
                    lo.index, slot.slot, slot.spell_id
                ));
            }
        }
    }

    let mut new_lines: Vec<String> = before.iter().map(|s| s.to_string()).collect();
    new_lines.extend(body);
    new_lines.extend(after.iter().map(|s| s.to_string()));
    let mut new_text = new_lines.join("\n");
    new_text.push('\n');
    new_text
}

/// why: overwrites the currently loaded file in place.
pub fn save_spellbook(
    base_dir: &Path,
    file: &str,
    loadouts: &[SpellLoadoutDto],
) -> Result<(), String> {
    validate_loadouts_shape(loadouts)?;

    let path = uifiles::ui_file_path(base_dir, file).map_err(|e| e.to_string())?;
    let original = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;

    // why: last-known-good copy before eqlp touches a real game config
    // file -- overwritten every save, so it's always "the version before
    // eqlp's most recent edit," not a growing pile
    let backup_path = PathBuf::from(format!("{}.eqlp-backup", path.display()));
    std::fs::write(&backup_path, &original).map_err(|e| e.to_string())?;

    let new_text = splice_spell_loadouts_block(&original, loadouts);
    std::fs::write(&path, new_text).map_err(|e| e.to_string())
}

/// why: real "save as" -- forks the current file pair (hotbuttons +
/// its `UI_`-prefixed layout counterpart) under a new `<Character>_
/// <Zone>` name, with the edited loadouts spliced in. The layout file
/// (window position/size, never spell contents) copies over verbatim --
/// there's nothing loadout-related in it to edit. Refuses to clobber an
/// existing pair: this is a fork, never an overwrite (that's what
/// save_spellbook is for).
pub fn save_spellbook_as(
    base_dir: &Path,
    source_file: &str,
    new_stem: &str,
    loadouts: &[SpellLoadoutDto],
) -> Result<String, String> {
    validate_loadouts_shape(loadouts)?;

    // why: the game's own naming scheme is exactly two `_`-delimited,
    // underscore-free segments (see uifiles.rs's name_pattern) -- a
    // stem that doesn't fit would produce a file the real client's own
    // UI file picker wouldn't recognize as a character/zone pair.
    match new_stem.split_once('_') {
        Some((c, z)) if !c.is_empty() && !z.is_empty() && !z.contains('_') => {}
        _ => {
            return Err(format!(
                "\"{new_stem}\" doesn't match the game's own <Character>_<Zone> naming -- exactly one underscore, no others"
            ));
        }
    }

    let new_hotbuttons = format!("{new_stem}_LO1.ini");
    let new_layout = format!("UI_{new_stem}_LO1.ini");
    let new_hotbuttons_path =
        uifiles::ui_file_path(base_dir, &new_hotbuttons).map_err(|e| e.to_string())?;
    let new_layout_path =
        uifiles::ui_file_path(base_dir, &new_layout).map_err(|e| e.to_string())?;
    if new_hotbuttons_path.exists() || new_layout_path.exists() {
        return Err(format!(
            "\"{new_stem}\" already has a saved file -- pick a different name"
        ));
    }

    let source_path = uifiles::ui_file_path(base_dir, source_file).map_err(|e| e.to_string())?;
    let original = std::fs::read_to_string(&source_path).map_err(|e| e.to_string())?;
    let new_text = splice_spell_loadouts_block(&original, loadouts);
    std::fs::write(&new_hotbuttons_path, new_text).map_err(|e| e.to_string())?;

    // why: best-effort -- the source may genuinely have no paired layout
    // file yet (the game only writes UI_ once a window's actually been
    // moved), missing one is not a reason to fail the whole save
    if let Ok(source_layout_path) = uifiles::ui_file_path(base_dir, &format!("UI_{source_file}")) {
        if let Ok(layout_text) = std::fs::read_to_string(&source_layout_path) {
            let _ = std::fs::write(&new_layout_path, layout_text);
        }
    }

    Ok(new_hotbuttons)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("eqlp-spellbookfiles-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// why: real shape, confirmed against the reference log's own
    /// install folder -- id/name pairs on `^`-delimited lines, extra
    /// fields after name ignored
    fn write_spells_us(dir: &Path) {
        std::fs::write(
            dir.join("spells_us.txt"),
            "652^Obscure^0^more^fields^here\n60^Resist Fire^0\n10^Augmentation^0\n",
        )
        .unwrap();
    }

    fn minimal_loadouts_ini() -> String {
        let mut s = String::from("[HotButtons]\nPage1Button1=H1,@-1,0,0,,\n[SpellLoadouts]\n");
        s.push_str("SpellLoadout1.inuse=1\nSpellLoadout1.name=buff-Others\n");
        for slot in 1..=MAX_SLOTS {
            let id = match slot {
                1 => 652,
                2 => 60,
                3 => 10,
                _ => -1,
            };
            s.push_str(&format!("SpellLoadout1.slot{slot}={id}\n"));
        }
        for idx in 2..=MAX_LOADOUTS {
            s.push_str(&format!("SpellLoadout{idx}.inuse=0\n"));
        }
        s.push_str("[HotButtons2]\nPage1Button1=H2,@-1,0,0,,\n");
        s
    }

    #[test]
    fn a_real_loadout_resolves_its_slot_names_and_catalog_links() {
        let dir = scratch_dir("load");
        write_spells_us(&dir);
        std::fs::write(
            dir.join("Manipulator_rivervale_LO1.ini"),
            minimal_loadouts_ini(),
        )
        .unwrap();

        let sb = load_spellbook(&dir, "Manipulator_rivervale_LO1.ini").unwrap();
        assert_eq!(sb.loadouts.len(), MAX_LOADOUTS as usize);

        let lo1 = &sb.loadouts[0];
        assert!(lo1.in_use);
        assert_eq!(lo1.name.as_deref(), Some("buff-Others"));
        assert_eq!(lo1.slots.len(), MAX_SLOTS as usize);
        assert_eq!(lo1.slots[0].spell_id, 652);
        assert_eq!(lo1.slots[0].name.as_deref(), Some("Obscure"));
        assert!(
            lo1.slots[0].catalog_id.is_some(),
            "Obscure is a real packs/spells.json entry"
        );
        assert_eq!(lo1.slots[3].spell_id, -1, "slot4 was never set -- empty");
        assert_eq!(lo1.slots[3].name, None);

        let lo2 = &sb.loadouts[1];
        assert!(!lo2.in_use);
        assert_eq!(lo2.name, None);
        assert!(lo2.slots.is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn saving_only_touches_the_spell_loadouts_block() {
        let dir = scratch_dir("save");
        write_spells_us(&dir);
        std::fs::write(
            dir.join("Manipulator_rivervale_LO1.ini"),
            minimal_loadouts_ini(),
        )
        .unwrap();

        let mut sb = load_spellbook(&dir, "Manipulator_rivervale_LO1.ini").unwrap();
        sb.loadouts[0].name = Some("Renamed Loadout".to_string());
        sb.loadouts[0].slots[3].spell_id = 60; // fill the empty slot4 with Resist Fire

        save_spellbook(&dir, "Manipulator_rivervale_LO1.ini", &sb.loadouts).unwrap();

        let written = std::fs::read_to_string(dir.join("Manipulator_rivervale_LO1.ini")).unwrap();
        assert!(written.contains("[HotButtons]"));
        assert!(written.contains("Page1Button1=H1,@-1,0,0,,"));
        assert!(written.contains("[HotButtons2]"));
        assert!(written.contains("Page1Button1=H2,@-1,0,0,,"));
        assert!(written.contains("SpellLoadout1.name=Renamed Loadout"));
        assert!(written.contains("SpellLoadout1.slot4=60"));

        let backup = std::fs::read_to_string(format!(
            "{}.eqlp-backup",
            dir.join("Manipulator_rivervale_LO1.ini").display()
        ))
        .unwrap();
        assert!(
            backup.contains("SpellLoadout1.name=buff-Others"),
            "backup keeps the pre-edit content"
        );

        // why: round-trips clean -- the edit sticks, the rest of the file is intact
        let reloaded = load_spellbook(&dir, "Manipulator_rivervale_LO1.ini").unwrap();
        assert_eq!(
            reloaded.loadouts[0].name.as_deref(),
            Some("Renamed Loadout")
        );
        assert_eq!(reloaded.loadouts[0].slots[3].spell_id, 60);
        assert_eq!(
            reloaded.loadouts[0].slots[3].name.as_deref(),
            Some("Resist Fire")
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn save_as_forks_the_file_pair_under_a_new_name_leaving_the_source_untouched() {
        let dir = scratch_dir("save_as");
        write_spells_us(&dir);
        std::fs::write(
            dir.join("Manipulator_rivervale_LO1.ini"),
            minimal_loadouts_ini(),
        )
        .unwrap();
        std::fs::write(
            dir.join("UI_Manipulator_rivervale_LO1.ini"),
            "[Main]\nUISkin=default_modern\n",
        )
        .unwrap();

        let mut sb = load_spellbook(&dir, "Manipulator_rivervale_LO1.ini").unwrap();
        sb.loadouts[0].name = Some("Forked Loadout".to_string());

        let new_file = save_spellbook_as(
            &dir,
            "Manipulator_rivervale_LO1.ini",
            "Alt_rivervale",
            &sb.loadouts,
        )
        .unwrap();
        assert_eq!(new_file, "Alt_rivervale_LO1.ini");

        let forked = load_spellbook(&dir, "Alt_rivervale_LO1.ini").unwrap();
        assert_eq!(forked.loadouts[0].name.as_deref(), Some("Forked Loadout"));

        let forked_layout = std::fs::read_to_string(dir.join("UI_Alt_rivervale_LO1.ini")).unwrap();
        assert!(
            forked_layout.contains("UISkin=default_modern"),
            "layout file copied verbatim"
        );

        let source = load_spellbook(&dir, "Manipulator_rivervale_LO1.ini").unwrap();
        assert_eq!(
            source.loadouts[0].name.as_deref(),
            Some("buff-Others"),
            "source file left untouched"
        );

        assert!(
            save_spellbook_as(
                &dir,
                "Manipulator_rivervale_LO1.ini",
                "Alt_rivervale",
                &sb.loadouts
            )
            .is_err(),
            "refuses to clobber an existing pair"
        );
        assert!(
            save_spellbook_as(
                &dir,
                "Manipulator_rivervale_LO1.ini",
                "NoZoneSeparator",
                &sb.loadouts
            )
            .is_err(),
            "refuses a stem with no <Character>_<Zone> separator"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn save_rejects_a_malformed_shape_rather_than_writing_it() {
        let dir = scratch_dir("reject");
        write_spells_us(&dir);
        std::fs::write(
            dir.join("Manipulator_rivervale_LO1.ini"),
            minimal_loadouts_ini(),
        )
        .unwrap();
        let sb = load_spellbook(&dir, "Manipulator_rivervale_LO1.ini").unwrap();

        let too_few = &sb.loadouts[..MAX_LOADOUTS as usize - 1];
        assert!(save_spellbook(&dir, "Manipulator_rivervale_LO1.ini", too_few).is_err());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn resolve_spell_id_is_case_insensitive_and_real() {
        let dir = scratch_dir("resolve");
        write_spells_us(&dir);
        assert_eq!(resolve_spell_id(&dir, "obscure"), Some(652));
        assert_eq!(resolve_spell_id(&dir, "OBSCURE"), Some(652));
        assert_eq!(resolve_spell_id(&dir, "Not A Real Spell"), None);
        std::fs::remove_dir_all(&dir).ok();
    }
}
