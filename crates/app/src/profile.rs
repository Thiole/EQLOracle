//! why: persisted per-character class profile -- fallback for
//! `commands::find_zone_route` when live replay hasn't confirmed a
//! configuration yet; live evidence always wins once it exists.
//!
//! Keyed by character name -- one install may see multiple characters,
//! a saved profile must never leak between them. Classes only, no level
//! (a real-time fact, always read from the live session instead).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CharacterProfile {
    /// why: alphabetical, top live configuration as of last write
    pub classes: Vec<String>,
}

const FILE_NAME: &str = "profiles.json";

fn profiles_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    Ok(dir.join(FILE_NAME))
}

/// why: missing or unreadable both mean "no profile saved for anyone yet"
fn load_all(app: &AppHandle) -> HashMap<String, CharacterProfile> {
    profiles_path(app)
        .ok()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// why: None covers no-file/no-entry/unreadable alike -- nothing to fall back to
pub fn for_character(app: &AppHandle, character: &str) -> Option<CharacterProfile> {
    load_all(app).remove(character)
}

/// why: writes only on real change -- called every tick, avoids disk churn
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
    crate::diskwrite::write_atomic(&path, &json).map_err(|e| e.to_string())?;
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

    /// why: two characters' saved profiles must never merge or leak
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
