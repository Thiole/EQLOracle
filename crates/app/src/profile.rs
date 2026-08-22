//! Persisted per-character class profile -- the saved-fallback half of
//! `preferences::Preferences::save_profile` (see that field's own doc for
//! the full policy: when it's off, this file is never read or written at
//! all; when it's on, this is *only* ever a fallback for `commands::
//! find_zone_route`'s own `player_classes`, used exclusively when the
//! current session's live replay hasn't itself confirmed a configuration
//! for "You" yet -- live evidence always wins the instant it exists).
//!
//! Keyed by character name (`tail_worker::TailStatus::character`, the
//! same identity `identity_from_filename` already derives from whichever
//! log file is newest) rather than one flat file, since one install can
//! genuinely see more than one character over time -- a saved profile for
//! "Aeliana" must never leak in as a guess for "Borgak".
//!
//! Deliberately holds *only* classes -- no level, no zone, no anything
//! that's a real-time fact rather than an inference. Level in particular
//! changes constantly and is never persisted here even though `classdetect`
//! reports it alongside a configuration; see `commands::find_zone_route`'s
//! own doc for why level always comes from the live session, never this
//! file.
//!
//! Same persistence shape every other file in this module family uses
//! (`app_config_dir`, a JSON file, load-on-read/save-on-write) -- see
//! `preferences.rs`/`settings.rs`/`config.rs`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CharacterProfile {
    /// Alphabetical, same convention `classdetect::Detector` already
    /// groups by -- this character's own top (most zone-visits) live
    /// configuration as of the last time `save_if_changed` wrote it.
    pub classes: Vec<String>,
}

const FILE_NAME: &str = "profiles.json";

fn profiles_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    Ok(dir.join(FILE_NAME))
}

/// Missing or unreadable both read as "no profile saved for anyone yet" --
/// same stance every sibling file in this module takes.
fn load_all(app: &AppHandle) -> HashMap<String, CharacterProfile> {
    profiles_path(app)
        .ok()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// `None` covers "no file yet", "file exists but not for this character",
/// and "unreadable" alike -- all three mean the same thing to a caller
/// that's only ever using this as a fallback: nothing to fall back to.
pub fn for_character(app: &AppHandle, character: &str) -> Option<CharacterProfile> {
    load_all(app).remove(character)
}

/// Overwrites `character`'s saved profile only when `classes` actually
/// differs from what's already on disk -- called from `tail_worker::
/// emit_tick` on every tick while `save_profile` is on, so a no-op write
/// every few seconds through an ordinary play session would otherwise be
/// pure disk churn for no reason. Returns whether it actually wrote.
pub fn save_if_changed(
    app: &AppHandle,
    character: &str,
    classes: &[String],
) -> Result<bool, String> {
    let mut all = load_all(app);
    if all.get(character).is_some_and(|p| p.classes == classes) {
        return Ok(false);
    }
    all.insert(
        character.to_string(),
        CharacterProfile {
            classes: classes.to_vec(),
        },
    );
    let path = profiles_path(app)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_vec_pretty(&all).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_serde() {
        let p = CharacterProfile {
            classes: vec![
                "Enchanter".to_string(),
                "Magician".to_string(),
                "Wizard".to_string(),
            ],
        };
        let json = serde_json::to_string(&p).unwrap();
        let back: CharacterProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);
    }

    /// The keyed-by-character shape this module is built around: two
    /// different characters' saved profiles must never merge or leak into
    /// each other.
    #[test]
    fn two_characters_keep_independent_profiles_in_the_same_file() {
        let raw = serde_json::json!({
            "Aeliana": {"classes": ["Cleric", "Paladin", "Warrior"]},
            "Borgak": {"classes": ["Necromancer", "Shadow Knight", "Wizard"]},
        });
        let all: HashMap<String, CharacterProfile> = serde_json::from_value(raw).unwrap();
        assert_eq!(all["Aeliana"].classes, vec!["Cleric", "Paladin", "Warrior"]);
        assert_eq!(
            all["Borgak"].classes,
            vec!["Necromancer", "Shadow Knight", "Wizard"]
        );
    }

    #[test]
    fn an_empty_json_object_has_no_profile_for_anyone() {
        let all: HashMap<String, CharacterProfile> = serde_json::from_str("{}").unwrap();
        assert!(all.is_empty());
    }
}
