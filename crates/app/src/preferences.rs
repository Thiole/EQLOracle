//! why: persisted general UI preferences, not tied to any one module --
//! notification volume, wiki era filter, save_profile opt-in. Not
//! notification-specific (that's `settings::NotificationSettings`), and
//! not per-character (that's `profile.rs`).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

/// why: a widget's last window position. Captured once, in
/// commands::set_overlay_locked, at the moment it's re-locked after
/// being dragged -- a mid-drag nudge that's never re-locked doesn't
/// half-persist. Logical pixels, same space as WebviewWindowBuilder::
/// position/inner_size, DPI-correct with no extra conversion. Backend-
/// only: never round-tripped through PreferencesDto, see
/// set_preferences' doc for why an unrelated change can't wipe it.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct OverlayPosition {
    pub x: f64,
    pub y: f64,
}

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

/// why: CC Tracker's only size preset a fresh install has ever seen --
/// see commands::cc_tracker_dims's own doc for what each preset maps to
fn default_cc_tracker_size() -> String {
    "small".to_string()
}

/// why: the second opacity knob. overlay_<widget>_opacity above is
/// background-only (rgba alpha on the panel; text stays fully
/// readable). This is a CSS `opacity` on the whole outer element, so
/// text/icons fade with everything else. Defaults fully opaque (1.0) --
/// an extra knob, not a new default look.
fn default_overall_opacity() -> f64 {
    1.0
}

/// why: the Skill Tracker's 4 baked-in status pseudo-entries -- distinct
/// from any real spell/ability name a track button could add. "Charmed"
/// not "Charm" -- real bug, caught live: Enchanter's own level 11 spell
/// really is named exactly "Charm", so casting it created a cooldown
/// entry that collided with this pseudo-entry, showing a nonsensical
/// "Charm: READY" cooldown row alongside the real "Charm: ACTIVE/Broke"
/// status row. Same reasoning already applied to "Invisible" (not
/// "Invisibility", also a real spell name) -- every one of these 4 must
/// never match a name skilltracker.rs could independently learn.
fn default_tracked_skills() -> Vec<String> {
    ["Charmed", "Invisible", "Hide", "Sneak"]
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
    /// why: the app version whose "what's new" the user has acknowledged;
    /// None on a fresh install (whatsnew.rs)
    #[serde(default)]
    pub last_seen_version: Option<String>,
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
    /// why: see default_overall_opacity's own doc -- the SEPARATE
    /// "everything" fade, not the panel-only one above. 1.0 (fully
    /// opaque) by default.
    #[serde(default = "default_overall_opacity")]
    pub overlay_dps_meter_overall_opacity: f64,
    /// why: the Skill Tracker widget's own opacity -- see
    /// overlay_dps_meter_opacity's own doc, same pattern. Covers all
    /// three of its sections (status effects, skill cooldowns, target
    /// effects) -- one window, one panel, one alpha, same as any other
    /// overlay widget here.
    #[serde(default = "default_overlay_opacity")]
    pub overlay_skill_tracker_opacity: f64,
    /// why: see overlay_dps_meter_overall_opacity's own doc -- same
    /// "everything" fade, this widget's own.
    #[serde(default = "default_overall_opacity")]
    pub overlay_skill_tracker_overall_opacity: f64,
    /// why: the Drop Watch widget's own opacity -- see
    /// overlay_dps_meter_opacity's own doc, same pattern. See
    /// dropwatch.rs's own doc for what this widget shows.
    #[serde(default = "default_overlay_opacity")]
    pub overlay_drop_watch_opacity: f64,
    /// why: see overlay_dps_meter_overall_opacity's own doc -- same
    /// "everything" fade, this widget's own.
    #[serde(default = "default_overall_opacity")]
    pub overlay_drop_watch_overall_opacity: f64,
    /// why: the CC Tracker widget's own opacity (Root/Stun/Fear squares)
    /// -- see overlay_dps_meter_opacity's own doc, same pattern. Its own
    /// widget, not a Skill Tracker section -- see CCTrackerWidget.
    /// svelte's own doc.
    #[serde(default = "default_overlay_opacity")]
    pub overlay_cc_tracker_opacity: f64,
    /// why: see overlay_dps_meter_overall_opacity's own doc -- same
    /// "everything" fade, this widget's own.
    #[serde(default = "default_overall_opacity")]
    pub overlay_cc_tracker_overall_opacity: f64,
    /// why: the Session widget's own opacity (AA/levels/plat rates +
    /// mote strip) -- see overlay_dps_meter_opacity's own doc, same pattern.
    #[serde(default = "default_overlay_opacity")]
    pub overlay_session_opacity: f64,
    /// why: see overlay_dps_meter_overall_opacity's own doc -- same
    /// "everything" fade, this widget's own.
    #[serde(default = "default_overall_opacity")]
    pub overlay_session_overall_opacity: f64,
    /// why: the Group Buff Tracker's own opacities -- same pattern
    #[serde(default = "default_overlay_opacity")]
    pub overlay_group_buffs_opacity: f64,
    #[serde(default = "default_overall_opacity")]
    pub overlay_group_buffs_overall_opacity: f64,
    /// why: "small"/"medium"/"large" -- CC Tracker's own layout knob, not
    /// shared by any other widget today. A plain String, not a Rust enum
    /// -- same "unrecognized value falls back, no hard error" contract
    /// as `theme` above (validated on the frontend by ccSize.ts's own
    /// asCcSize(), and on this side by commands::cc_tracker_dims()).
    #[serde(default = "default_cc_tracker_size")]
    pub overlay_cc_tracker_size: String,
    /// why: which entries show in the Skill Tracker overlay -- real
    /// tracked abilities/spells (added via a "track" button, none by
    /// default) plus the 4 baked-in status pseudo-entries (Charmed/
    /// Invisible/Hide/Sneak), which start present -- pre-added rather
    /// than opt-in. Removable the same way any tracked skill is.
    #[serde(default = "default_tracked_skills")]
    pub tracked_skills: Vec<String>,
    /// why: separate from tracked_skills -- a target effect isn't a
    /// cooldown/READY row, it only shows in the target-effects section
    /// (landed? duration left?). Empty by default, unlike tracked_skills
    /// (nothing here is baked in). Entry point is Spellbook's "Overlay
    /// spell tracking" section.
    #[serde(default)]
    pub tracked_target_effects: Vec<String>,
    /// why: item names the player wants a heads-up on when currently
    /// fighting a mob known to drop one -- see dropwatch.rs's own doc.
    /// Empty by default; entry points are Sky Quests' own material chips
    /// and Primary Class Unlocks' reward materials, same "track" shape
    /// as tracked_skills/tracked_target_effects.
    #[serde(default)]
    pub tracked_drop_items: Vec<String>,
    /// why: how many of each tracked item were already accounted for the
    /// last time its "remove from Drop Watch?" prompt was shown or
    /// auto-dismissed (dropwatch.rs's own `TrackedLootDto.count`, not a
    /// timestamp -- a fresh loot after a decline is still a fresh prompt).
    /// Persisted, not session-only: without this, restarting the app
    /// would re-backfill the same old loot line and prompt about an item
    /// gotten days ago all over again. Seeded to the real current count
    /// the moment an item is newly tracked, so tracking something you
    /// already have doesn't immediately prompt either.
    #[serde(default)]
    pub tracked_drop_seen_counts: HashMap<String, u64>,
    /// why: see OverlayPosition's own doc. Keyed by widget name (same
    /// "dps_meter"/"skill_tracker" strings commands::overlay_label
    /// already uses), empty until a widget's been dragged and re-locked
    /// at least once.
    #[serde(default)]
    pub overlay_positions: HashMap<String, OverlayPosition>,
    /// why: Character Planner's hand-set race -- the log never states
    /// race, so losing it every launch means re-picking it every launch.
    /// Backend-only, same never-round-tripped-through-PreferencesDto
    /// stance as overlay_positions (see set_preferences); reachable via
    /// get_planner_state/set_planner_state only.
    #[serde(default)]
    pub planner_race: Option<String>,
    /// why: ONLY the levels the user typed over the estimate -- presence
    /// in this map IS the "user updated" flag the UI shows. Estimated
    /// levels are recomputed fresh each launch and never stored;
    /// "Estimate levels" clears this map (the explicit reset lever).
    /// Backend-only, same as planner_race.
    #[serde(default)]
    pub planner_levels: HashMap<String, u8>,
    /// why: real epoch ms (`Date.now()` on the frontend, which owns this
    /// entirely -- backend just stores whatever it's given), refreshed
    /// roughly every 5 minutes while Drop Watch has anything tracked
    /// (see dropWatchLoot.ts's own doc), not on every poll -- "ish" is
    /// fine, a few minutes of blind spot on an ungraceful close is a
    /// real accepted trade-off, not a bug. What "new" means for the
    /// "remove from Drop Watch?" prompt: a loot event timestamped after
    /// the *last* value this ever held, not a fixed window back from
    /// whenever the app happens to next check -- a real gap in an
    /// earlier version, which used a flat 30s-from-now window and would
    /// wrongly miss a genuine pickup that happened while the app
    /// (not the game) was briefly closed. None until Drop Watch has
    /// tracked anything at least once.
    #[serde(default)]
    pub drop_watch_checkpoint_ms: Option<i64>,
    // why: no "is this widget currently on" field -- live session state,
    // not a style preference. An earlier version persisted on/off and
    // reopened widgets automatically, trusting stale state the same way
    // save_profile's doc warns against for class detection. Every
    // launch starts with every widget off; opacity still carries over
    // once turned back on.
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            last_seen_version: None,
            volume: default_volume(),
            era: None,
            save_profile: false,
            update_channel: UpdateChannel::default(),
            theme: default_theme(),
            overlay_dps_meter_opacity: default_overlay_opacity(),
            overlay_dps_meter_overall_opacity: default_overall_opacity(),
            overlay_skill_tracker_opacity: default_overlay_opacity(),
            overlay_skill_tracker_overall_opacity: default_overall_opacity(),
            overlay_drop_watch_opacity: default_overlay_opacity(),
            overlay_drop_watch_overall_opacity: default_overall_opacity(),
            overlay_cc_tracker_opacity: default_overlay_opacity(),
            overlay_cc_tracker_overall_opacity: default_overall_opacity(),
            overlay_session_opacity: default_overlay_opacity(),
            overlay_session_overall_opacity: default_overall_opacity(),
            overlay_group_buffs_opacity: default_overlay_opacity(),
            overlay_group_buffs_overall_opacity: default_overall_opacity(),
            overlay_cc_tracker_size: default_cc_tracker_size(),
            tracked_skills: default_tracked_skills(),
            tracked_target_effects: Vec::new(),
            tracked_drop_items: Vec::new(),
            tracked_drop_seen_counts: HashMap::new(),
            overlay_positions: HashMap::new(),
            planner_race: None,
            planner_levels: HashMap::new(),
            drop_watch_checkpoint_ms: None,
        }
    }
}

const FILE_NAME: &str = "preferences.json";

fn preferences_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    Ok(dir.join(FILE_NAME))
}

/// why: measured inefficiency, full-app walk 2026-08-29 -- load() was a
/// disk read + JSON parse on EVERY call, and callers call it per
/// parse-tick from the worker (emit_tick) plus per tick from EACH open
/// overlay window's own refreshPrefs -- 5+ redundant disk reads per
/// tick at steady state. One process, one config file: a module cache
/// filled on first load and updated by save() makes every later load a
/// clone. The file is never edited externally while running (and if it
/// were, the old code's racing readers had no coherent answer either).
static CACHE: std::sync::RwLock<Option<Preferences>> = std::sync::RwLock::new(None);

/// why: missing or unreadable both mean "nothing saved yet", fall back quietly
pub fn load(app: &AppHandle) -> Preferences {
    if let Some(p) = CACHE.read().unwrap().as_ref() {
        return p.clone();
    }
    let loaded: Preferences = preferences_path(app)
        .ok()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    *CACHE.write().unwrap() = Some(loaded.clone());
    loaded
}

pub fn save(app: &AppHandle, prefs: &Preferences) -> Result<(), String> {
    let path = preferences_path(app)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_vec_pretty(prefs).map_err(|e| e.to_string())?;
    crate::diskwrite::write_atomic(&path, &json).map_err(|e| e.to_string())?;
    // why: cache updated only after the write succeeded -- a failed save
    // must not leave readers seeing state the disk doesn't have
    *CACHE.write().unwrap() = Some(prefs.clone());
    Ok(())
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
        assert_eq!(p.overlay_dps_meter_overall_opacity, 1.0);
        assert_eq!(p.overlay_skill_tracker_opacity, 0.85);
        assert_eq!(p.overlay_skill_tracker_overall_opacity, 1.0);
        assert_eq!(
            p.tracked_skills,
            vec!["Charmed", "Invisible", "Hide", "Sneak"],
            "on by default, not opt-in like a real tracked skill -- see default_tracked_skills' own doc"
        );
        assert!(
            p.tracked_target_effects.is_empty(),
            "nothing baked in -- a per-target effect is always an opt-in pick, unlike the 4 status pseudo-entries above"
        );
        assert_eq!(p.overlay_drop_watch_opacity, 0.85);
        assert_eq!(p.overlay_drop_watch_overall_opacity, 1.0);
        assert_eq!(p.overlay_cc_tracker_opacity, 0.85);
        assert_eq!(p.overlay_cc_tracker_overall_opacity, 1.0);
        assert_eq!(p.overlay_cc_tracker_size, "small");
        assert!(
            p.tracked_drop_items.is_empty(),
            "nothing baked in -- always an opt-in pick"
        );
        assert!(
            p.tracked_drop_seen_counts.is_empty(),
            "no baseline recorded until something's actually tracked"
        );
        assert!(
            p.overlay_positions.is_empty(),
            "no widget has a saved position until it's been dragged and re-locked at least once"
        );
        assert_eq!(p.planner_race, None, "race is hand-set, never guessed");
        assert!(
            p.planner_levels.is_empty(),
            "empty until the user types over an estimate -- presence IS the user-updated flag"
        );
    }

    #[test]
    fn round_trips_through_serde() {
        let mut overlay_positions = HashMap::new();
        overlay_positions.insert(
            "dps_meter".to_string(),
            OverlayPosition { x: 12.5, y: -4.0 },
        );
        let mut tracked_drop_seen_counts = HashMap::new();
        tracked_drop_seen_counts.insert("Light Woolen Mask".to_string(), 2u64);
        let p = Preferences {
            last_seen_version: None,
            volume: 42,
            era: Some("All".to_string()),
            save_profile: true,
            update_channel: UpdateChannel::Beta,
            theme: "claude".to_string(),
            overlay_dps_meter_opacity: 0.4,
            overlay_dps_meter_overall_opacity: 0.7,
            overlay_skill_tracker_opacity: 0.6,
            overlay_skill_tracker_overall_opacity: 0.9,
            overlay_drop_watch_opacity: 0.5,
            overlay_drop_watch_overall_opacity: 0.8,
            overlay_cc_tracker_opacity: 0.3,
            overlay_cc_tracker_overall_opacity: 0.6,
            overlay_session_opacity: 0.45,
            overlay_session_overall_opacity: 0.65,
            overlay_group_buffs_opacity: 0.45,
            overlay_group_buffs_overall_opacity: 0.65,
            overlay_cc_tracker_size: "large".to_string(),
            tracked_skills: vec!["Kick".to_string(), "Backstab".to_string()],
            tracked_target_effects: vec!["Tashania".to_string()],
            tracked_drop_items: vec!["Light Woolen Mask".to_string()],
            tracked_drop_seen_counts,
            overlay_positions,
            planner_race: Some("Halfling".to_string()),
            planner_levels: HashMap::from([("Wizard".to_string(), 34u8)]),
            drop_watch_checkpoint_ms: Some(1_700_000_000_000),
        };
        let json = serde_json::to_string(&p).unwrap();
        let back: Preferences = serde_json::from_str(&json).unwrap();
        assert_eq!(back.volume, 42);
        assert_eq!(back.era.as_deref(), Some("All"));
        assert!(back.save_profile);
        assert_eq!(back.update_channel, UpdateChannel::Beta);
        assert_eq!(back.theme, "claude");
        assert_eq!(back.overlay_dps_meter_opacity, 0.4);
        assert_eq!(back.overlay_dps_meter_overall_opacity, 0.7);
        assert_eq!(back.overlay_skill_tracker_opacity, 0.6);
        assert_eq!(back.overlay_skill_tracker_overall_opacity, 0.9);
        assert_eq!(back.overlay_drop_watch_opacity, 0.5);
        assert_eq!(back.overlay_drop_watch_overall_opacity, 0.8);
        assert_eq!(back.overlay_cc_tracker_opacity, 0.3);
        assert_eq!(back.overlay_cc_tracker_overall_opacity, 0.6);
        assert_eq!(back.overlay_cc_tracker_size, "large");
        assert_eq!(back.tracked_skills, vec!["Kick", "Backstab"]);
        assert_eq!(back.tracked_target_effects, vec!["Tashania"]);
        assert_eq!(back.tracked_drop_items, vec!["Light Woolen Mask"]);
        assert_eq!(
            back.tracked_drop_seen_counts.get("Light Woolen Mask"),
            Some(&2)
        );
        let pos = back
            .overlay_positions
            .get("dps_meter")
            .expect("round-tripped");
        assert_eq!((pos.x, pos.y), (12.5, -4.0));
        assert_eq!(back.planner_race.as_deref(), Some("Halfling"));
        assert_eq!(back.planner_levels.get("Wizard"), Some(&34));
        assert_eq!(back.drop_watch_checkpoint_ms, Some(1_700_000_000_000));
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
        assert_eq!(back.overlay_dps_meter_overall_opacity, 1.0);
        assert_eq!(back.overlay_skill_tracker_opacity, 0.85);
        assert_eq!(back.overlay_skill_tracker_overall_opacity, 1.0);
        assert_eq!(back.overlay_drop_watch_opacity, 0.85);
        assert_eq!(back.overlay_drop_watch_overall_opacity, 1.0);
        assert_eq!(back.overlay_cc_tracker_opacity, 0.85);
        assert_eq!(back.overlay_cc_tracker_overall_opacity, 1.0);
        assert_eq!(back.overlay_cc_tracker_size, "small");
        assert_eq!(
            back.tracked_skills,
            vec!["Charmed", "Invisible", "Hide", "Sneak"]
        );
        assert!(back.tracked_target_effects.is_empty());
        assert!(back.tracked_drop_items.is_empty());
        assert!(back.tracked_drop_seen_counts.is_empty());
        assert!(back.overlay_positions.is_empty());
    }
}
