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

/// why: this app's own original identity -- see themes.css's own doc for
/// where the other presets come from and why "eqlp" mirrors :root's own
/// base values rather than being a special-cased empty string
fn default_theme() -> String {
    "eqlp".to_string()
}

/// why: mostly opaque but still see-through by default -- a fresh install
/// should read cleanly over the game, not vanish into it
fn default_overlay_opacity() -> f64 {
    0.85
}

/// why: the Skill Tracker's 4 baked-in status pseudo-entries -- distinct
/// from any real spell/ability name a track button could add (in
/// particular "Invisible" not "Invisibility", the real spell -- that
/// exact string is trackable on its own via Spellbook, and must never
/// collide with "am I currently invisible" from effects.rs)
fn default_tracked_skills() -> Vec<String> {
    ["Charm", "Invisible", "Hide", "Sneak"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// why: which release channel this install checks for updates against --
/// `public` = the `latest` GitHub release (main, deliberate releases
/// only), `beta` = the `testing` release (every push to `testing`,
/// prerelease). See `.github/workflows/3-release.yml`'s own doc for the
/// two tags this maps to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum UpdateChannel {
    #[default]
    Public,
    Beta,
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
    /// why: defaults Public -- an install never opts into prerelease
    /// builds without asking
    #[serde(default)]
    pub update_channel: UpdateChannel,
    /// why: a slug matching one of themes.css's own `[data-theme="X"]`
    /// blocks -- not validated against a known list here, an unknown/old
    /// slug from a downgraded install just falls back to no visible
    /// override (browser ignores an unmatched attribute selector),
    /// never a hard error
    #[serde(default = "default_theme")]
    pub theme: String,
    /// why: each overlay widget owns its own opacity, not one shared
    /// window-wide setting -- more widgets are coming (a party tracker
    /// is the next one planned), each independently placed and
    /// independently see-through. 0.0 (invisible) to 1.0 (fully opaque)
    /// -- this widget's own panel background alpha, not a native
    /// window-opacity call (see windowcap.rs's own doc on why the window
    /// itself is just `transparent: true` and content controls how
    /// see-through it reads). Named overlay_<widget>_opacity so the next
    /// widget follows the same pattern instead of inventing a new shape.
    #[serde(default = "default_overlay_opacity")]
    pub overlay_dps_meter_opacity: f64,
    /// why: the Skill Tracker widget's own opacity -- see
    /// overlay_dps_meter_opacity's own doc, same pattern. Covers all
    /// three of its sections (status effects, skill cooldowns, target
    /// effects) -- one window, one panel, one alpha, same as any other
    /// overlay widget here.
    #[serde(default = "default_overlay_opacity")]
    pub overlay_skill_tracker_opacity: f64,
    /// why: which entries actually show in the Skill Tracker overlay --
    /// both real tracked abilities/spells (added via a "track" button in
    /// Spellbook/Combat, nothing by default -- a fresh install doesn't
    /// know which skills this character even has) AND the 4 baked-in
    /// status pseudo-entries (Charm/Invisible/Hide/Sneak), which start
    /// present -- Spencer's own ask: "not always track, but on by
    /// default", i.e. real list members like anything else here, just
    /// pre-added instead of opt-in. Removable the same way any tracked
    /// skill is (the overlay's own listbox).
    #[serde(default = "default_tracked_skills")]
    pub tracked_skills: Vec<String>,
    // why: no "is this widget / the overlay window currently on" field --
    // deliberately not a style preference to remember, it's live session
    // state. Caught live: an earlier version persisted the window's own
    // on/off and reopened it automatically on every launch, silently
    // trusting stale state the same way save_profile's own doc
    // explicitly warns against for class detection. Every launch starts
    // with every widget off; each widget's own opacity still carries
    // over once it's turned back on.
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            volume: default_volume(),
            era: None,
            save_profile: false,
            update_channel: UpdateChannel::default(),
            theme: default_theme(),
            overlay_dps_meter_opacity: default_overlay_opacity(),
            overlay_skill_tracker_opacity: default_overlay_opacity(),
            tracked_skills: default_tracked_skills(),
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
        assert_eq!(
            p.update_channel,
            UpdateChannel::Public,
            "never opts into prerelease builds without being asked"
        );
        assert_eq!(
            p.theme, "eqlp",
            "this app's own identity, not an upstream preset"
        );
        assert_eq!(p.overlay_dps_meter_opacity, 0.85);
        assert_eq!(p.overlay_skill_tracker_opacity, 0.85);
        assert_eq!(
            p.tracked_skills,
            vec!["Charm", "Invisible", "Hide", "Sneak"],
            "on by default, not opt-in like a real tracked skill -- see default_tracked_skills' own doc"
        );
    }

    #[test]
    fn round_trips_through_serde() {
        let p = Preferences {
            volume: 42,
            era: Some("All".to_string()),
            save_profile: true,
            update_channel: UpdateChannel::Beta,
            theme: "claude".to_string(),
            overlay_dps_meter_opacity: 0.4,
            overlay_skill_tracker_opacity: 0.6,
            tracked_skills: vec!["Kick".to_string(), "Backstab".to_string()],
        };
        let json = serde_json::to_string(&p).unwrap();
        let back: Preferences = serde_json::from_str(&json).unwrap();
        assert_eq!(back.volume, 42);
        assert_eq!(back.era.as_deref(), Some("All"));
        assert!(back.save_profile);
        assert_eq!(back.update_channel, UpdateChannel::Beta);
        assert_eq!(back.theme, "claude");
        assert_eq!(back.overlay_dps_meter_opacity, 0.4);
        assert_eq!(back.overlay_skill_tracker_opacity, 0.6);
        assert_eq!(back.tracked_skills, vec!["Kick", "Backstab"]);
    }

    /// why: an old/partial file must still load via #[serde(default)]
    #[test]
    fn an_empty_json_object_still_loads_with_defaults() {
        let back: Preferences = serde_json::from_str("{}").unwrap();
        assert_eq!(back.volume, 100);
        assert_eq!(back.era, None);
        assert!(!back.save_profile);
        assert_eq!(back.update_channel, UpdateChannel::Public);
        assert_eq!(back.theme, "eqlp");
        assert_eq!(back.overlay_dps_meter_opacity, 0.85);
        assert_eq!(back.overlay_skill_tracker_opacity, 0.85);
        assert_eq!(
            back.tracked_skills,
            vec!["Charm", "Invisible", "Hide", "Sneak"]
        );
    }
}
