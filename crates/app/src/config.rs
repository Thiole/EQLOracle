//! Persisted app config: just the chosen log directory. Written once at
//! first-launch setup, read on every subsequent start so the picker screen
//! is a one-time cost.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub log_dir: PathBuf,
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
