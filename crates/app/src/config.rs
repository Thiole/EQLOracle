//! Persisted app config: just the chosen game install folder. Written once
//! at first-launch setup, read on every subsequent start so the picker
//! screen is a one-time cost.
//!
//! Deliberately the game's *base* install folder (e.g. ".../EverQuest
//! Legends/"), not the `Logs` subfolder directly -- confirmed against a
//! real install: `/outputfile inventory` writes its dump one level *above*
//! `Logs`, in the base folder itself, so a picker scoped to `Logs` alone
//! can never reach it. `AppConfig::log_dir` derives the tail target from
//! this single stored path instead, so there's only one directory to ever
//! pick and only one place the base-to-Logs relationship is expressed.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub base_dir: PathBuf,
}

impl AppConfig {
    /// Where the tail worker actually watches -- EQ's own fixed layout,
    /// not configurable and not separately stored, so there's no second
    /// path that could ever drift out of sync with `base_dir`.
    pub fn log_dir(&self) -> PathBuf {
        self.base_dir.join("Logs")
    }
}

const FILE_NAME: &str = "config.json";

fn config_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    Ok(dir.join(FILE_NAME))
}

/// `None` covers "never configured" and "config unreadable" alike -- both
/// mean the same thing to the caller: show the first-launch screen.
pub fn load(app: &AppHandle) -> Option<AppConfig> {
    let path = config_path(app).ok()?;
    let bytes = std::fs::read(&path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub fn save(app: &AppHandle, cfg: &AppConfig) -> Result<(), String> {
    let path = config_path(app)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_vec_pretty(cfg).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())
}
