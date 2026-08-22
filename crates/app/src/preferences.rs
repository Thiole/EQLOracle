//! Persisted general UI preferences -- not notification-specific (that's
//! `settings::NotificationSettings`'s own job), just standing choices
//! about how the app should behave that aren't tied to any one module:
//! notification volume (for once sound playback itself is ported to this
//! UI -- not yet, see `Preferences::volume`'s own doc), which wiki era
//! `gearplanner`/Game Data should filter to, and whether class detection
//! should carry a saved profile across restarts (`Preferences::
//! save_profile`'s own doc; the profile itself lives in `profile.rs`, a
//! separate file since it's per-character data, not a standing UI
//! choice). Same persistence shape `settings.rs` already uses
//! (`app_config_dir`, a JSON file, load-on-read/save-on-write), kept in
//! its own file rather than folded into `notifications.json` since these
//! aren't notification preferences.

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
    /// Default `false`: every launch replays the whole log and lets
    /// `classdetect` reconfirm classes purely from what that replay
    /// actually sees, same as always -- see `history.rs`'s own doc for
    /// why a clean re-derive every start is the deliberate default here
    /// (a stale carried-over class record from a build with different
    /// detection logic already caused a real, confirmed bug once:
    /// ~2,900 loadouts claiming 4-10 simultaneous classes).
    ///
    /// `true` opts into a narrower, deliberately-scoped exception to that
    /// stance: `tail_worker::emit_tick` keeps writing this character's own
    /// live-resolved top configuration to `profile.rs` as it's
    /// (re)confirmed each session, and `commands::find_zone_route` falls
    /// back to that saved configuration *only* when the current session's
    /// own live replay hasn't confirmed one yet for "You" -- e.g. early in
    /// a short session, before enough casts have landed this run to
    /// reconfirm what a longer previous session already established. Live
    /// evidence still always wins the moment it exists; this never
    /// silently pins a stale answer over a fresher, fully-reconfirmed one,
    /// and it never touches anything that isn't itself an inference (raw
    /// log facts -- zone, `/loc`, damage, level -- are never overridden).
    /// The whole point is that a good detector rarely needs this in
    /// practice; it exists for the gap while one hasn't caught up yet.
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

    /// A settings file written before `volume`/`era`/`save_profile`
    /// existed (or with any of them simply absent) must still load, not
    /// fail -- `#[serde(default)]` on all three is what makes an old/
    /// partial file safe.
    #[test]
    fn an_empty_json_object_still_loads_with_defaults() {
        let back: Preferences = serde_json::from_str("{}").unwrap();
        assert_eq!(back.volume, 100);
        assert_eq!(back.era, None);
        assert!(!back.save_profile);
    }
}
