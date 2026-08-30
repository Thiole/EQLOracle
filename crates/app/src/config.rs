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

/// why: "figure it out for them" -- the player's own ask, and the popup
/// text already warns about the exact mistake this repairs: picking the
/// `Logs` folder instead of the install folder that contains it. A dir
/// named Logs (or holding eqlog_*.txt files) resolves to its parent; a
/// candidate then counts as a real install if it has a `Logs` subdir or
/// `eqgame.exe` (a fresh install may not have Logs until /log first
/// runs). Anything else errors WITHOUT saving -- the old code persisted
/// any directory at all, wedging setup behind a broken config.
pub fn normalize_base_dir(dir: &std::path::Path) -> Result<PathBuf, String> {
    let looks_like_install =
        |d: &std::path::Path| d.join("Logs").is_dir() || d.join("eqgame.exe").is_file();
    let is_logs_dir = dir
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.eq_ignore_ascii_case("logs"))
        || std::fs::read_dir(dir).is_ok_and(|entries| {
            entries.flatten().any(|e| {
                e.file_name()
                    .to_str()
                    .is_some_and(|n| n.starts_with("eqlog_") && n.ends_with(".txt"))
            })
        });
    if is_logs_dir {
        if let Some(parent) = dir.parent() {
            if looks_like_install(parent) {
                return Ok(parent.to_path_buf());
            }
        }
    }
    if looks_like_install(dir) {
        return Ok(dir.to_path_buf());
    }
    Err(format!(
        "{} doesn't look like the install folder -- pick the folder that directly contains `Logs` (it usually also holds eqgame.exe)",
        dir.display()
    ))
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
    crate::diskwrite::write_atomic(&path, &json).map_err(|e| e.to_string())
}

#[cfg(test)]
mod normalize_tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("eqlp-normtest-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// why: the player's ask verbatim -- "if someone accidentally picks
    /// log folder, can you figure it out to pick the base folder for
    /// them instead?"
    #[test]
    fn picking_the_logs_folder_resolves_to_its_parent_install() {
        let base = scratch("logs-pick");
        std::fs::create_dir_all(base.join("Logs")).unwrap();
        std::fs::write(base.join("eqgame.exe"), b"x").unwrap();
        assert_eq!(normalize_base_dir(&base.join("Logs")).unwrap(), base);
    }

    /// why: a renamed/moved log folder still identifies by its eqlog files
    #[test]
    fn a_dir_full_of_eqlog_files_also_resolves_to_its_parent() {
        let base = scratch("eqlog-pick");
        let logs = base.join("MyLogs");
        std::fs::create_dir_all(&logs).unwrap();
        std::fs::write(logs.join("eqlog_Kaeus_rivervale.txt"), b"x").unwrap();
        std::fs::write(base.join("eqgame.exe"), b"x").unwrap();
        assert_eq!(normalize_base_dir(&logs).unwrap(), base);
    }

    /// why: correct pick passes through untouched; a fresh install with
    /// no Logs yet counts via eqgame.exe
    #[test]
    fn a_real_install_folder_is_accepted_as_is() {
        let base = scratch("good-pick");
        std::fs::create_dir_all(base.join("Logs")).unwrap();
        assert_eq!(normalize_base_dir(&base).unwrap(), base);

        let fresh = scratch("fresh-pick");
        std::fs::write(fresh.join("eqgame.exe"), b"x").unwrap();
        assert_eq!(normalize_base_dir(&fresh).unwrap(), fresh);
    }

    /// why: the Windows report's root -- the old code saved ANY existing
    /// directory, wedging setup behind a broken config; an unrecognizable
    /// pick must error and save nothing
    #[test]
    fn an_unrelated_folder_errors_instead_of_persisting() {
        let d = scratch("wrong-pick");
        std::fs::create_dir_all(d.join("random")).unwrap();
        assert!(normalize_base_dir(&d).is_err());
    }
}
