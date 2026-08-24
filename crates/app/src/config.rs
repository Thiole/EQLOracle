//! why: persisted app config -- the chosen game install folder
//!
//! Stores the game's *base* folder, not `Logs` directly -- confirmed
//! `/outputfile inventory` writes one level above `Logs`, so a `Logs`-only
//! picker can never reach it. `log_dir` derives the tail path from this.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub base_dir: PathBuf,
}

impl AppConfig {
    /// why: EQ's fixed layout, not stored separately so it can't drift
    pub fn log_dir(&self) -> PathBuf {
        self.base_dir.join("Logs")
    }
}

const FILE_NAME: &str = "config.json";

fn config_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    Ok(dir.join(FILE_NAME))
}

/// why: None covers both "never configured" and "unreadable" -- same UI
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
