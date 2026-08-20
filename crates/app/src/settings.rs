//! Persisted notification preferences: per-kind enabled/disabled, and an
//! optional user-uploaded sound overriding the frontend's own synthesized
//! default. Same persistence shape `config::AppConfig` already uses
//! (`app_config_dir`, a JSON file, load-on-read/save-on-write) but a
//! separate file: `AppConfig` is specifically the game install folder,
//! written once at first-launch setup, and overloading its own doc/flow
//! with an unrelated concern would blur what's a genuinely different
//! kind of setting -- this one has no first-launch gate at all, and can
//! change any number of times per session from the Settings module.
//!
//! Deliberately *not* subject to `history`'s "purges on every app start"
//! stance -- a notification preference is a standing choice about how the
//! app should behave, not a fact about a specific play session, so it
//! persists the same way the install folder does.

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NotificationSettings {
    /// kind -> enabled. A kind with no entry reads as enabled -- see
    /// `is_enabled`'s doc for why default-on, not default-off.
    #[serde(default)]
    enabled: HashMap<String, bool>,
    /// kind -> stored sound filename (relative to the sounds directory,
    /// not a full path -- see `store_custom_sound`). Absent means "use
    /// the frontend's synthesized default for this kind", not "muted";
    /// muting is `enabled`'s job, entirely separate from which sound
    /// plays when it *is* enabled.
    #[serde(default)]
    custom_sound: HashMap<String, String>,
}

impl NotificationSettings {
    /// Default-on: a brand-new install (or a fifth notification kind
    /// added later, with no entry yet in an existing settings file on
    /// disk) should announce itself rather than silently do nothing until
    /// the user finds the Settings module and opts in -- the whole point
    /// of shipping a default sound per kind.
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

/// Missing or unreadable both read as "no preferences saved yet" -- same
/// stance `config::load` takes, and the same reason: a fresh/corrupt file
/// should fall back to defaults quietly, not surface as an error to a
/// user who never touched this file directly.
pub fn load(app: &AppHandle) -> NotificationSettings {
    settings_path(app)
        .ok()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub fn save(app: &AppHandle, settings: &NotificationSettings) -> Result<(), String> {
    let path = settings_path(app)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_vec_pretty(settings).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())
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

/// Copies `source` (a file the user just picked via the OS file dialog)
/// into the app's own sounds directory, named after `kind` -- copied in,
/// not referenced by its original path, so later moving, renaming, or
/// deleting the source file the user picked from can't silently break a
/// notification that used to work. Overwrites any previous custom sound
/// for this same `kind` (one file per kind, not a history of past
/// uploads). Returns the stored filename -- what `NotificationSettings::
/// set_custom_sound` should be given, and what `custom_sound_data_url`
/// re-reads later.
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

/// The stored custom sound's bytes, base64-encoded as a `data:` URL the
/// frontend can hand straight to `new Audio(url)` -- an ordinary IPC
/// command return value, not a served asset, so this doesn't need the
/// asset-protocol/CSP scope no other command in this app needs either.
/// `None` for a kind with no custom sound, or one whose stored file has
/// gone missing on disk -- the frontend's own synthesized default covers
/// both cases identically, so this doesn't distinguish why.
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
