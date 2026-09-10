//! why: right-click "assign to player" -- a mob that is really someone's
//! pet, keyed by zone visit and name, kept in pet_owners.json so every
//! replay folds it the same way

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PetOwner {
    /// why: the zone visit index the UI selects by; None is the pre-zone bucket
    pub visit: Option<usize>,
    pub pet: String,
    pub owner: String,
}

/// why: right-click "hide" -- the row leaves both tables for that visit,
/// the data stays
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HiddenEntity {
    pub visit: Option<usize>,
    pub name: String,
}

const OWNERS_FILE: &str = "pet_owners.json";
const HIDDEN_FILE: &str = "hidden_entities.json";

fn path(app: &AppHandle, file: &str) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    Ok(dir.join(file))
}

/// why: missing or unreadable both mean "nothing set yet"
pub fn load_from<T: serde::de::DeserializeOwned>(path: &Path) -> Vec<T> {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save_to<T: Serialize>(path: &Path, list: &[T]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_vec_pretty(list).map_err(|e| e.to_string())?;
    crate::diskwrite::write_atomic(path, &json).map_err(|e| e.to_string())
}

/// why: both right-click overrides, loaded once and applied to every fresh replay
#[derive(Debug, Default, Clone)]
pub struct Overrides {
    pub owners: Vec<PetOwner>,
    pub hidden: Vec<HiddenEntity>,
}

pub fn load(app: &AppHandle) -> Overrides {
    Overrides {
        owners: path(app, OWNERS_FILE)
            .map(|p| load_from(&p))
            .unwrap_or_default(),
        hidden: path(app, HIDDEN_FILE)
            .map(|p| load_from(&p))
            .unwrap_or_default(),
    }
}

pub fn apply(ing: &mut crate::ingest::Ingest, o: &Overrides) {
    ing.set_manual_pet_owners(&o.owners);
    ing.set_hidden_entities(&o.hidden);
}

pub fn save_owners(app: &AppHandle, list: &[PetOwner]) -> Result<(), String> {
    save_to(&path(app, OWNERS_FILE)?, list)
}

pub fn save_hidden(app: &AppHandle, list: &[HiddenEntity]) -> Result<(), String> {
    save_to(&path(app, HIDDEN_FILE)?, list)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assignments_round_trip_through_the_file_and_a_missing_file_is_empty() {
        let dir = std::env::temp_dir().join(format!("eqlp-petowners-{}", std::process::id()));
        let file = dir.join("pet_owners.json");
        assert!(load_from::<PetOwner>(&file).is_empty());
        let list = vec![PetOwner {
            visit: Some(12),
            pet: "a fire elemental".to_string(),
            owner: "You".to_string(),
        }];
        save_to(&file, &list).unwrap();
        assert_eq!(load_from::<PetOwner>(&file), list);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
