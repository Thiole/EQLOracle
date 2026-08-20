//! Persisted general UI preferences -- not notification-specific (that's
//! `settings::NotificationSettings`'s own job), just standing choices
//! about how the app should behave that aren't tied to any one module:
//! notification volume (for once sound playback itself is ported to this
//! UI -- not yet, see `Preferences::volume`'s own doc) and which wiki era
//! `gearplanner`/Game Data should filter to. Same persistence shape
//! `settings.rs` already uses (`app_config_dir`, a JSON file, load-on-
//! read/save-on-write), kept in its own file rather than folded into
//! `notifications.json` since these aren't notification preferences.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

fn default_volume() -> u8 {
    100
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preferences {
    /// 0-100. Not yet wired to any actual sound playback -- the new UI
    /// hasn't ported notification sounds at all yet (see `ui/app-legacy/
    /// app.js`'s `playNotificationSound` for the feature this will
    /// eventually feed into). Saved now so the control exists ahead of
    /// that port rather than being added twice.
    #[serde(default = "default_volume")]
    pub volume: u8,
    /// An `gearplanner::ERA_ORDER` name, the literal `"All"` (no era
    /// ceiling at all), or `None` -- meaning no preference saved yet,
    /// which every era-aware call (`gearplanner::in_era`) reads as
    /// `gearplanner::CURRENT_ERA`. Never normalized to a concrete era
    /// string at save time: leaving it `None` until the user actually
    /// picks something means a later bump to `CURRENT_ERA` (the server
    /// progressing) changes a fresh install's default automatically,
    /// rather than baking in whatever era happened to be current when
    /// this file was first written.
    #[serde(default)]
    pub era: Option<String>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            volume: default_volume(),
            era: None,
        }
    }
}

const FILE_NAME: &str = "preferences.json";

fn preferences_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    Ok(dir.join(FILE_NAME))
}

/// Missing or unreadable both read as "nothing saved yet" -- same stance
/// `settings::load`/`config::load` already take, for the same reason: a
/// fresh install or a corrupt file should fall back to defaults quietly.
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
        assert_eq!(p.era, None, "None -- in_era's own CURRENT_ERA default, not a baked-in era string");
    }

    #[test]
    fn round_trips_through_serde() {
        let p = Preferences {
            volume: 42,
            era: Some("All".to_string()),
        };
        let json = serde_json::to_string(&p).unwrap();
        let back: Preferences = serde_json::from_str(&json).unwrap();
        assert_eq!(back.volume, 42);
        assert_eq!(back.era.as_deref(), Some("All"));
    }

    /// A settings file written before `volume`/`era` existed (or with
    /// either key simply absent) must still load, not fail -- `#[serde(
    /// default)]` on both fields is what makes an old/partial file safe.
    #[test]
    fn an_empty_json_object_still_loads_with_defaults() {
        let back: Preferences = serde_json::from_str("{}").unwrap();
        assert_eq!(back.volume, 100);
        assert_eq!(back.era, None);
    }
}
