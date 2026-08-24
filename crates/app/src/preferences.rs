//! why: persisted general UI preferences, not tied to any one module --
//! notification volume, wiki era filter, save_profile opt-in. Not
//! notification-specific (that's `settings::NotificationSettings`), and
//! not per-character (that's `profile.rs`).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

fn default_volume() -> u8 {
    100
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preferences {
    /// why: 0-100, not yet wired to playback -- saved ahead of that port
    #[serde(default = "default_volume")]
    pub volume: u8,
    /// why: None means no preference -- reads as `gearplanner::CURRENT_ERA`
    /// so a later era bump updates a fresh install's default automatically
    #[serde(default)]
    pub era: Option<String>,
    /// why: default false, every launch reconfirms classes fresh -- a
    /// stale carried-over record once caused ~2,900 bogus multi-class
    /// loadouts. When true, `profile.rs` is only a fallback until this
    /// session's own live replay confirms a configuration; live evidence
    /// always wins once it exists.
    #[serde(default)]
    pub save_profile: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            volume: default_volume(),
            era: None,
            save_profile: false,
        }
    }
}

const FILE_NAME: &str = "preferences.json";

fn preferences_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    Ok(dir.join(FILE_NAME))
}

/// why: missing or unreadable both mean "nothing saved yet", fall back quietly
pub fn load(app: &AppHandle) -> Preferences {
    preferences_path(app)
        .ok()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub fn save(app: &AppHandle, prefs: &Preferences) -> Result<(), String> {
    let path = preferences_path(app)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_vec_pretty(prefs).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_install_defaults_to_full_volume_and_no_era_preference() {
        let p = Preferences::default();
        assert_eq!(p.volume, 100);
        assert_eq!(
            p.era, None,
            "None -- in_era's own CURRENT_ERA default, not a baked-in era string"
        );
        assert!(
            !p.save_profile,
            "off by default -- every launch infers fresh unless the user opts in"
        );
    }

    #[test]
    fn round_trips_through_serde() {
        let p = Preferences {
            volume: 42,
            era: Some("All".to_string()),
            save_profile: true,
        };
        let json = serde_json::to_string(&p).unwrap();
        let back: Preferences = serde_json::from_str(&json).unwrap();
        assert_eq!(back.volume, 42);
        assert_eq!(back.era.as_deref(), Some("All"));
        assert!(back.save_profile);
    }

    /// why: an old/partial file must still load via #[serde(default)]
    #[test]
    fn an_empty_json_object_still_loads_with_defaults() {
        let back: Preferences = serde_json::from_str("{}").unwrap();
        assert_eq!(back.volume, 100);
        assert_eq!(back.era, None);
        assert!(!back.save_profile);
    }
}
