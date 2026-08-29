//! why: persisted notification prefs -- per-kind enabled + custom sound
//!
//! Separate file from `config::AppConfig` (that's the install folder,
//! first-launch-only) since this changes any number of times per session
//! from Settings. Not subject to `history`'s purge-on-start -- a standing
//! behavior choice, not a play-session fact.

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NotificationSettings {
    /// why: kind -> enabled; no entry reads as enabled, see `is_enabled`
    #[serde(default)]
    enabled: HashMap<String, bool>,
    /// why: kind -> stored filename; absent means use synthesized default,
    /// not muted -- muting is `enabled`'s job entirely
    #[serde(default)]
    custom_sound: HashMap<String, String>,
}

impl NotificationSettings {
    /// why: default-on -- a new kind should announce itself, not wait for opt-in
    pub fn is_enabled(&self, kind: &str) -> bool {
        self.enabled.get(kind).copied().unwrap_or(true)
    }

    pub fn set_enabled(&mut self, kind: &str, on: bool) {
        self.enabled.insert(kind.to_string(), on);
    }

    pub fn custom_sound(&self, kind: &str) -> Option<&str> {
        self.custom_sound.get(kind).map(String::as_str)
    }

    pub fn set_custom_sound(&mut self, kind: &str, filename: Option<String>) {
        match filename {
            Some(f) => {
                self.custom_sound.insert(kind.to_string(), f);
            }
            None => {
                self.custom_sound.remove(kind);
            }
        }
    }
}

const FILE_NAME: &str = "notifications.json";

fn settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    Ok(dir.join(FILE_NAME))
}

/// why: same per-tick disk-read caching as preferences::load -- see its
/// own doc; emit_tick loads this on every tick carrying a notification
static CACHE: std::sync::RwLock<Option<NotificationSettings>> = std::sync::RwLock::new(None);

/// why: missing or unreadable both mean "no preferences saved yet", quiet fallback
pub fn load(app: &AppHandle) -> NotificationSettings {
    if let Some(s) = CACHE.read().unwrap().as_ref() {
        return s.clone();
    }
    let loaded: NotificationSettings = settings_path(app)
        .ok()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    *CACHE.write().unwrap() = Some(loaded.clone());
    loaded
}

pub fn save(app: &AppHandle, settings: &NotificationSettings) -> Result<(), String> {
    let path = settings_path(app)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_vec_pretty(settings).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())?;
    // why: cache follows a successful write only, same as preferences::save
    *CACHE.write().unwrap() = Some(settings.clone());
    Ok(())
}

fn sounds_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| e.to_string())?
        .join("sounds");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// why: copies picked file in (not referenced by path) so later moves/
/// deletes can't break it; overwrites any previous sound for this kind
pub fn store_custom_sound(app: &AppHandle, kind: &str, source: &Path) -> Result<String, String> {
    let ext = source.extension().and_then(|e| e.to_str()).unwrap_or("mp3");
    let filename = format!("{kind}.{ext}");
    let dest = sounds_dir(app)?.join(&filename);
    std::fs::copy(source, &dest).map_err(|e| format!("couldn't copy {}: {e}", source.display()))?;
    Ok(filename)
}

pub fn delete_custom_sound(app: &AppHandle, filename: &str) {
    if let Ok(dir) = sounds_dir(app) {
        let _ = std::fs::remove_file(dir.join(filename));
    }
}

/// why: base64 `data:` URL, ordinary IPC return value not a served asset;
/// None for no-custom-sound or missing file alike -- default covers both
pub fn custom_sound_data_url(
    app: &AppHandle,
    kind: &str,
    settings: &NotificationSettings,
) -> Option<String> {
    let filename = settings.custom_sound(kind)?;
    let path = sounds_dir(app).ok()?.join(filename);
    let bytes = std::fs::read(&path).ok()?;
    let mime = match Path::new(filename).extension().and_then(|e| e.to_str()) {
        Some("wav") => "audio/wav",
        Some("ogg") => "audio/ogg",
        Some("m4a") => "audio/mp4",
        _ => "audio/mpeg",
    };
    Some(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kind_with_no_saved_entry_defaults_to_enabled() {
        let s = NotificationSettings::default();
        assert!(s.is_enabled(crate::notifications::LEVEL_UP));
    }

    #[test]
    fn set_enabled_round_trips_through_serde() {
        let mut s = NotificationSettings::default();
        s.set_enabled(crate::notifications::CHARM_BROKEN, false);
        let json = serde_json::to_string(&s).unwrap();
        let back: NotificationSettings = serde_json::from_str(&json).unwrap();
        assert!(!back.is_enabled(crate::notifications::CHARM_BROKEN));
        assert!(
            back.is_enabled(crate::notifications::LEVEL_UP),
            "an untouched kind should still default to enabled"
        );
    }

    #[test]
    fn custom_sound_round_trips_and_clears() {
        let mut s = NotificationSettings::default();
        assert_eq!(s.custom_sound(crate::notifications::AA_GAINED), None);
        s.set_custom_sound(
            crate::notifications::AA_GAINED,
            Some("aa_gained.wav".to_string()),
        );
        assert_eq!(
            s.custom_sound(crate::notifications::AA_GAINED),
            Some("aa_gained.wav")
        );
        s.set_custom_sound(crate::notifications::AA_GAINED, None);
        assert_eq!(s.custom_sound(crate::notifications::AA_GAINED), None);
    }
}
