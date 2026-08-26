//! why: the IPC surface -- toolbar/setup commands plus read-only queries
//! against the shared `Ingest`, no reparsing. Live updates go over
//! `parse-tick`/`parse-error` events from `tail_worker` instead.

use crate::aadata;
use crate::character::{self, CharacterEstimateDto};
use crate::chat::{self, ChatMessageDto, PmThreadDto};
use crate::combat::{
    self, AllyDto, ClassConfigurationsDto, CombatSummaryDto, EncounterDetailDto, EncounterDto,
    EntityStateDto, FightTimelineDto, ZoneEncounterDto, ZoneVisitDto,
};
use crate::config::{self, AppConfig};
use crate::debugview::{self, DebugEncounterDto, UnmatchedCoverageDto};
use crate::dpscalc::{self, DamageSpellDto};
use crate::gearplanner::{self, InventoryDumpDto, ItemDto, SlotRecommendationDto};
use crate::history::{self, ParseRecord};
use crate::ingest::LineCounts;
use crate::inventory;
use crate::mapsdata;
use crate::mobalias;
use crate::monsters::{self, LootEventDto, MobDto, MobStatsDto};
use crate::notifications;
use crate::npcdata;
use crate::overview::{self, SessionDto};
use crate::pathfind;
use crate::preferences::{self, Preferences};
use crate::profile;
use crate::progression::{self, AaLogDto, SpellbookEntryDto};
use crate::raiding::{self, RaidRowDto};
use crate::routing;
use crate::settings;
use crate::skyquests;
use crate::spellbookfiles;
use crate::spelldata;
use crate::spelleffect;
use crate::stackingdata;
use crate::state::AppState;
use crate::tail_worker::{self, TailStatus};
use crate::uifiles;
use crate::updater;
use crate::windowcap::{self, WindowCapability, WindowCapabilityDto};
use crate::zonedata;
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};

#[derive(Debug, Clone, Serialize)]
pub struct StatusDto {
    pub configured: bool,
    pub status: TailStatus,
    pub counts: LineCounts,
}

#[tauri::command]
pub fn get_status(state: State<AppState>) -> StatusDto {
    StatusDto {
        configured: state.config.lock().unwrap().is_some(),
        status: state.status.lock().unwrap().clone(),
        counts: state.ingest.lock().unwrap().counts.clone(),
    }
}

/// why: native folder picker, None on cancel (not an error). Async
/// callback API, not blocking_pick_folder -- Linux's GTK/portal dialog
/// doesn't reliably mesh with a blocked command thread.
#[tauri::command]
pub async fn pick_log_directory(app: AppHandle) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Select your EverQuest Legends install folder")
        .pick_folder(move |folder| {
            let _ = tx.send(folder);
        });
    rx.await.ok().flatten().map(|p| p.to_string())
}

/// why: persists the base install folder, (re)starts the tail worker;
/// same path for first-launch and "change folder" later. Worker points
/// at `cfg.log_dir()`, not `path` directly.
#[tauri::command]
pub fn set_log_directory(
    app: AppHandle,
    state: State<AppState>,
    path: String,
) -> Result<StatusDto, String> {
    let dir = PathBuf::from(&path);
    if !dir.is_dir() {
        return Err(format!("{path} is not a directory"));
    }

    let cfg = AppConfig { base_dir: dir };
    config::save(&app, &cfg)?;
    let log_dir = cfg.log_dir();
    *state.config.lock().unwrap() = Some(cfg);

    if let Some(old) = state.worker.lock().unwrap().take() {
        old.stop();
    }
    let handle = tail_worker::spawn(
        app.clone(),
        log_dir,
        state.ingest.clone(),
        state.status.clone(),
    );
    *state.worker.lock().unwrap() = Some(handle);

    Ok(StatusDto {
        configured: true,
        status: state.status.lock().unwrap().clone(),
        counts: state.ingest.lock().unwrap().counts.clone(),
    })
}

/// why: Combat module's first dropdown -- zone visits, newest first, fight counts

#[tauri::command]
pub fn list_zone_visits(state: State<AppState>) -> Vec<ZoneVisitDto> {
    combat::list_zone_visits(&state.ingest.lock().unwrap())
}

/// why: Combat module's second dropdown, defaults to the whole list --
/// a rendering cost the frontend virtualizes, not a fetch one.
/// `zone_visit`: None = no filter, -1 = "Unknown" bucket, else a visit index.
#[tauri::command]
pub fn list_encounters(
    state: State<AppState>,
    zone_visit: Option<i64>,
    offset: Option<usize>,
    limit: Option<usize>,
) -> Vec<EncounterDto> {
    combat::list_encounters(
        &state.ingest.lock().unwrap(),
        zone_visit,
        offset.unwrap_or(0),
        limit.unwrap_or(usize::MAX),
    )
}

/// why: zone page's recent fights, keyed by `zone_id` not display name;
/// cheap on its own, no damage/drops -- see `get_encounter_detail`
#[tauri::command]
pub fn list_zone_encounters(
    state: State<AppState>,
    zone_id: String,
    limit: Option<usize>,
) -> Vec<ZoneEncounterDto> {
    combat::list_zone_encounters(&state.ingest.lock().unwrap(), &zone_id, limit.unwrap_or(30))
}

/// why: damage/drops fetched separately so the initial list never waits on them

#[tauri::command]
pub fn get_encounter_detail(
    state: State<AppState>,
    encounter_id: u32,
) -> Option<EncounterDetailDto> {
    combat::encounter_detail(&state.ingest.lock().unwrap(), encounter_id)
}

/// why: NPC page's kills/pulls totals plus recent fights

#[tauri::command]
pub fn get_mob_stats(state: State<AppState>, mob_name: String) -> MobStatsDto {
    monsters::mob_stats(&state.ingest.lock().unwrap(), &mob_name)
}

#[tauri::command]
pub fn list_mob_encounters(
    state: State<AppState>,
    mob_name: String,
    limit: Option<usize>,
) -> Vec<ZoneEncounterDto> {
    combat::list_mob_encounters(
        &state.ingest.lock().unwrap(),
        &mob_name,
        limit.unwrap_or(30),
    )
}

/// why: Debug module's table -- recent encounters with raw and resolved zone tags

#[tauri::command]
pub fn list_debug_encounters(
    state: State<AppState>,
    limit: Option<usize>,
) -> Vec<DebugEncounterDto> {
    debugview::list_debug_encounters(&state.ingest.lock().unwrap(), limit.unwrap_or(100))
}

/// why: Debug module's "Unparsed" tab -- unmatched shapes ranked by count

#[tauri::command]
pub fn get_unmatched_coverage(state: State<AppState>, top: Option<usize>) -> UnmatchedCoverageDto {
    debugview::unmatched_coverage(&state.ingest.lock().unwrap(), top.unwrap_or(100))
}

/// why: Combat module's primary view -- allies sorted by total damage descending

#[tauri::command]
pub fn list_allies(
    state: State<AppState>,
    zone_visit: Option<i64>,
    encounter_id: Option<u32>,
) -> Vec<AllyDto> {
    combat::list_allies(&state.ingest.lock().unwrap(), zone_visit, encounter_id)
}

/// why: Combat module's drill-down -- one ally's breakdown, or the whole selection's

#[tauri::command]
pub fn get_combat_summary(
    state: State<AppState>,
    zone_visit: Option<i64>,
    encounter_id: Option<u32>,
    actor: Option<String>,
) -> CombatSummaryDto {
    combat::summarize(
        &state.ingest.lock().unwrap(),
        zone_visit,
        encounter_id,
        actor.as_deref(),
    )
}

/// Per-entity damage-over-time bars for one fight's scrub bar.
#[tauri::command]
pub fn get_fight_timeline(state: State<AppState>, encounter_id: u32) -> Option<FightTimelineDto> {
    combat::fight_timeline(&state.ingest.lock().unwrap(), encounter_id)
}

/// What clicking a point on the scrub bar shows: every entity's state and a
/// snapshot DPS reading as of that instant.
#[tauri::command]
pub fn get_fight_state_at(
    state: State<AppState>,
    encounter_id: u32,
    ts_ms: i64,
) -> Vec<EntityStateDto> {
    combat::fight_state_at(&state.ingest.lock().unwrap(), encounter_id, ts_ms)
}

/// why: every configuration for one entity, most zone visits first; empty if nothing confirmed yet

#[tauri::command]
pub fn get_class_configurations(state: State<AppState>, name: String) -> ClassConfigurationsDto {
    combat::class_configurations(&state.ingest.lock().unwrap(), &name)
}

/// why: Endgame's Raiding tab, curated list with confirmed kills/tiers/loot

#[tauri::command]
pub fn get_raids(state: State<AppState>) -> Vec<RaidRowDto> {
    raiding::list_raid_rows(&state.ingest.lock().unwrap())
}

/// why: "Sky - Primary Class Unlocks" tab -- final reward items only, not raw materials

#[tauri::command]
pub fn get_sky_class_unlocks(state: State<AppState>) -> Vec<skyquests::SkyClassUnlockDto> {
    let base_dir = state
        .config
        .lock()
        .unwrap()
        .as_ref()
        .map(|c| c.base_dir.clone());
    skyquests::list_class_unlocks(&state.ingest.lock().unwrap(), base_dir.as_deref())
}

/// why: "Sky - Quests" tab -- every material turn-in, full detail

#[tauri::command]
pub fn get_sky_quests(state: State<AppState>) -> Vec<skyquests::SkyClassDto> {
    let base_dir = state
        .config
        .lock()
        .unwrap()
        .as_ref()
        .map(|c| c.base_dir.clone());
    skyquests::list_quests(&state.ingest.lock().unwrap(), base_dir.as_deref())
}

/// why: drills from a configuration row down to its own zone visits

#[tauri::command]
pub fn get_configuration_zone_visits(
    state: State<AppState>,
    name: String,
    classes: Vec<String>,
    level_range: Option<(u8, u8)>,
) -> Vec<ZoneVisitDto> {
    combat::zone_visits_for_configuration(
        &state.ingest.lock().unwrap(),
        &name,
        &classes,
        level_range,
    )
}

/// why: read-only symbol lookup; None only before anything's been parsed
fn you_sym(ing: &crate::ingest::Ingest) -> Option<u32> {
    ing.store.names.get("You").map(|s| s.0)
}

/// why: past parses against `target`, newest first; re-resolves loadout
/// against live class evidence before returning, not the as-of-close snapshot
#[tauri::command]
pub fn get_mob_history(
    app: AppHandle,
    state: State<AppState>,
    target: String,
    confirmed_only: bool,
) -> Vec<ParseRecord> {
    let mut records = history::all(&app);
    let ing = state.ingest.lock().unwrap();
    if let Some(you) = you_sym(&ing) {
        history::refresh_loadouts(&mut records, &ing.classes, you);
    }
    drop(ing);
    history::mob_history_view(records, &target, confirmed_only)
}

/// why: past parses bundled by class combination -- avoids blending
/// playstyles into one number; same treatment as `get_mob_history`
#[tauri::command]
pub fn get_loadout_summary(
    app: AppHandle,
    state: State<AppState>,
    target: String,
    confirmed_only: bool,
) -> Vec<history::LoadoutSummary> {
    let mut records = history::for_target(&app, &target);
    let ing = state.ingest.lock().unwrap();
    if let Some(you) = you_sym(&ing) {
        history::refresh_loadouts(&mut records, &ing.classes, you);
    }
    drop(ing);
    if confirmed_only {
        records = history::only_confirmed_kills(records);
    }
    history::by_loadout(&records)
}

/// why: Overview module's session stats, scoped to "this session" not the whole log

#[tauri::command]
pub fn get_session(state: State<AppState>) -> SessionDto {
    overview::session(&state.ingest.lock().unwrap())
}

/// why: every AA purchase this session plus total spent; no UI consumes this yet

#[tauri::command]
pub fn get_aa_log(state: State<AppState>) -> AaLogDto {
    progression::aa_log(&state.ingest.lock().unwrap())
}

/// why: Character module's Spellbook subpage -- known spells enriched with catalog stats

#[tauri::command]
pub fn get_spellbook(state: State<AppState>) -> Vec<SpellbookEntryDto> {
    progression::spellbook(&state.ingest.lock().unwrap())
}

/// why: Spellbook builder's picker -- shows an already-ranked spell's real rank

#[tauri::command]
pub fn get_spell_ranks(state: State<AppState>) -> HashMap<String, u8> {
    progression::spell_ranks(&state.ingest.lock().unwrap())
}

/// why: every damage-capable spell, rank-adjusted, unfiltered -- caller applies its own filtering

#[tauri::command]
pub fn get_damage_spells(state: State<AppState>, assume_max_rank: bool) -> Vec<DamageSpellDto> {
    dpscalc::list_damage_spells(&state.ingest.lock().unwrap(), assume_max_rank)
}

/// why: Loot History module's one view -- mob types, kills, loot

#[tauri::command]
pub fn list_mobs(state: State<AppState>) -> Vec<MobDto> {
    monsters::list_mobs(&state.ingest.lock().unwrap())
}

/// why: Social tab's Guild sub-channel

#[tauri::command]
pub fn get_guild_chat(state: State<AppState>) -> Vec<ChatMessageDto> {
    chat::guild_chat(&state.ingest.lock().unwrap())
}

/// why: Social tab's Party sub-channel

#[tauri::command]
pub fn get_party_chat(state: State<AppState>) -> Vec<ChatMessageDto> {
    chat::party_chat(&state.ingest.lock().unwrap())
}

/// why: Social tab's Raid sub-channel

#[tauri::command]
pub fn get_raid_chat(state: State<AppState>) -> Vec<ChatMessageDto> {
    chat::raid_chat(&state.ingest.lock().unwrap())
}

/// why: Social tab's PM player list, most-recent-message first

#[tauri::command]
pub fn list_pm_threads(state: State<AppState>) -> Vec<PmThreadDto> {
    chat::pm_threads(&state.ingest.lock().unwrap())
}

/// why: one PM thread's whole history, oldest first

#[tauri::command]
pub fn get_pm_history(state: State<AppState>, player: String) -> Vec<ChatMessageDto> {
    chat::pm_history(&state.ingest.lock().unwrap(), &player)
}

/// why: Overlay tab's own capability check -- see windowcap.rs's own doc
/// on why this is asked, never assumed

#[tauri::command]
pub fn get_window_capability() -> WindowCapabilityDto {
    windowcap::detect()
}

/// why: the DPS meter overlay's whole data source, also usable for a
/// live preview inside the main window itself

#[tauri::command]
pub fn get_live_meter(state: State<AppState>) -> Option<combat::LiveMeterDto> {
    combat::live_meter(&state.ingest.lock().unwrap())
}

/// why: overlay's timed-effects widget -- same polled-on-tick shape as
/// get_live_meter, see effects.rs's own doc
#[tauri::command]
pub fn get_status_effects(state: State<AppState>) -> crate::effects::StatusEffectsDto {
    crate::effects::status_effects(&state.ingest.lock().unwrap())
}

/// why: Skill Tracker widget's own-cooldowns section -- see skilltracker.rs's own doc
#[tauri::command]
pub fn get_skill_status(state: State<AppState>) -> Vec<crate::skilltracker::SkillStatusDto> {
    crate::skilltracker::skill_status(&state.ingest.lock().unwrap())
}

/// why: Skill Tracker widget's target-effects section -- see targeteffects.rs's own doc
#[tauri::command]
pub fn get_target_effects(state: State<AppState>) -> crate::targeteffects::TargetEffectsDto {
    crate::targeteffects::target_effects(&state.ingest.lock().unwrap())
}

/// why: each widget is its own real OS window now, not content stacked
/// inside one shared overlay surface -- independently draggable,
/// independently closable, matches how every other per-widget thing
/// here already works (own enable, own opacity). `overlay-*` is a glob
/// entry in capabilities/default.json so a new widget never needs a
/// capabilities edit to get IPC.
fn overlay_label(widget: &str) -> String {
    format!("overlay-{widget}")
}

/// why: creates (or closes) this one widget's own floating window -- a
/// fresh capability check every time, never trusts a stale frontend
/// value, since the session's own display server can't change mid-run
/// but a stale cached capability shouldn't be trusted to open one
/// anyway.

#[tauri::command]
pub fn set_overlay_enabled(app: AppHandle, widget: String, enabled: bool) -> Result<(), String> {
    let label = overlay_label(&widget);
    if !enabled {
        if let Some(w) = app.get_webview_window(&label) {
            let _ = w.close();
        }
        return Ok(());
    }
    if app.get_webview_window(&label).is_some() {
        return Ok(()); // already open
    }
    let cap = windowcap::detect();
    if cap.capability == WindowCapability::Docked {
        return Err(cap
            .reason
            .unwrap_or_else(|| "Floating overlays aren't available in this session.".to_string()));
    }
    // why: one shared overlay.html bundle for every widget -- which one
    // to render is read from the window's own label at runtime (see
    // ui's currentOverlayWidget()), not a distinct HTML entry per widget
    let mut builder =
        WebviewWindowBuilder::new(&app, &label, WebviewUrl::App("overlay.html".into()))
            .title(format!("EQL Oracle Overlay -- {widget}"))
            .inner_size(360.0, 240.0)
            .transparent(true)
            .decorations(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .shadow(false);
    // why: Spencer's own ask -- "remembers where those windows were set
    // in previous runs, so they dont have to be moved every time".
    // See preferences::OverlayPosition's own doc for where this gets
    // captured (set_overlay_locked, below); absent until then, opening
    // at whatever the OS/window manager's own default position is,
    // same as always.
    if let Some(pos) = preferences::load(&app).overlay_positions.get(&widget) {
        builder = builder.position(pos.x, pos.y);
    }
    let window = builder.build().map_err(|e| e.to_string())?;
    // why: ClickThrough only -- Floating alone (never actually
    // reachable today, detect() only ever returns Docked or
    // ClickThrough, kept as its own tier for when finer Wayland
    // detection becomes possible) would still block clicks on the
    // game underneath it
    if cap.capability == WindowCapability::ClickThrough {
        let _ = window.set_ignore_cursor_events(true);
    }
    Ok(())
}

/// why: live-pushes to this widget's own open window -- a no-op, not an
/// error, when it isn't open; persistence is the caller's own
/// setPreferences call. The window is already widget-scoped by its own
/// label, so the event payload is just the number.

#[tauri::command]
pub fn set_overlay_opacity(app: AppHandle, widget: String, opacity: f64) {
    if let Some(w) = app.get_webview_window(&overlay_label(&widget)) {
        let _ = w.emit("overlay-opacity", opacity.clamp(0.0, 1.0));
    }
}

/// why: click-through (locked, the default -- see set_overlay_enabled)
/// makes the window impossible to drag into position at all, since every
/// click passes straight to the game underneath it. Unlocking briefly
/// (a real toggle in the Overlay tab) accepts clicks again so the panel
/// can be repositioned, and also turns real decorations back on -- a
/// live check against this exact real setup (XWayland via KWin) found
/// `data-tauri-drag-region`'s own move request silently doesn't move the
/// window there (a resize-border drag does), so the one mechanism every
/// window manager is guaranteed to support -- dragging a real title bar
/// -- is what's actually used, not the borderless trick. Per-widget,
/// same as everything else here -- each window is repositioned on its
/// own. A no-op if that widget's window isn't open, or if this
/// session's own capability never allowed click-through to begin with
/// (nothing to toggle back to)
///
/// Also where a widget's own new position gets saved -- Spencer's own
/// ask, see preferences::OverlayPosition's own doc. Captured exactly
/// once, right here, at the moment of RE-locking (the user's own real
/// "I'm done positioning this" signal) -- not continuously on every
/// move event mid-drag, so a window that was merely nudged but never
/// actually re-locked doesn't half-persist. Logical pixels, matching
/// WebviewWindowBuilder::position's own coordinate space exactly (see
/// set_overlay_enabled) -- outer_position() itself returns physical
/// pixels, converted here via the window's own real scale factor so
/// this is correct on a HiDPI display, not just assumed 1:1.

#[tauri::command]
pub fn set_overlay_locked(app: AppHandle, widget: String, locked: bool) -> Result<(), String> {
    if windowcap::detect().capability != WindowCapability::ClickThrough {
        return Ok(());
    }
    if let Some(w) = app.get_webview_window(&overlay_label(&widget)) {
        w.set_ignore_cursor_events(locked)
            .map_err(|e| e.to_string())?;
        w.set_decorations(!locked).map_err(|e| e.to_string())?;
        if locked {
            if let (Ok(pos), Ok(scale)) = (w.outer_position(), w.scale_factor()) {
                let logical = pos.to_logical::<f64>(scale);
                let mut prefs = preferences::load(&app);
                prefs.overlay_positions.insert(
                    widget,
                    preferences::OverlayPosition {
                        x: logical.x,
                        y: logical.y,
                    },
                );
                let _ = preferences::save(&app, &prefs);
            }
        }
    }
    Ok(())
}

/// why: Game Data's Zones tab, 117 zones small enough to ship whole

#[tauri::command]
pub fn list_zones() -> Vec<zonedata::Zone> {
    zonedata::zones().to_vec()
}

/// why: Game Data's NPCs tab, 6,532 entries, still one call not paginated

#[tauri::command]
pub fn list_npcs() -> Vec<npcdata::Npc> {
    npcdata::npcs().to_vec()
}

/// why: one table, ships whole -- no second copy hand-kept in sync on the frontend

#[tauri::command]
pub fn get_mob_aliases() -> Vec<(String, String)> {
    mobalias::all()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// why: Game Data's Spells tab, 1928 entries, one call unfiltered like `list_zones`

#[tauri::command]
pub fn list_spells() -> Vec<spelldata::Spell> {
    spelldata::spells().to_vec()
}

/// why: derived spell mechanics for the whole catalog, keyed by spell id, computed once and cached

#[tauri::command]
pub fn list_spell_effects() -> Vec<spelleffect::SpellEffectsEntry> {
    spelleffect::all_effects().to_vec()
}

/// why: spell -> stacking group id, for the spellbook suggester's
/// "never suggest two mutually-exclusive spells" check -- see stackingdata.rs

#[tauri::command]
pub fn get_spell_stacking_groups() -> HashMap<String, u32> {
    stackingdata::stacking_groups().clone()
}

/// why: Game Data's AAs tab, full 142-entry catalog -- distinct from `get_aa_log`'s own purchases

#[tauri::command]
pub fn list_aa() -> Vec<aadata::Aa> {
    aadata::aas().to_vec()
}

/// why: item page's "your history with this item" section

#[tauri::command]
pub fn get_item_loot_history(state: State<AppState>, item: String) -> Vec<LootEventDto> {
    monsters::item_loot_history(&state.ingest.lock().unwrap(), &item)
}

/// why: Character Planner's one call -- full attribute sheet + mana
/// estimate; `gear` summed by the frontend, this side never touches an
/// item. Stateless on purpose -- nothing persisted, fresh launch starts blank.
#[tauri::command]
pub fn get_character_estimate(
    race: String,
    classes: Vec<String>,
    class_levels: Vec<u8>,
    gear: Option<std::collections::HashMap<String, f64>>,
) -> Option<CharacterEstimateDto> {
    character::estimate(&race, &classes, &class_levels, &gear.unwrap_or_default())
}

/// why: Gear Planner pre-selects this instead of asking again; empty if nothing confirmed

#[tauri::command]
pub fn get_default_gear_classes(state: State<AppState>, name: String) -> Vec<String> {
    gearplanner::default_classes(&state.ingest.lock().unwrap(), &name)
}

/// why: item browser; `owned`/`owned_tier` are the frontend's already-
/// loaded dump fields passed back in -- this command has no dump of its own
#[tauri::command]
pub fn list_gear_items(
    classes: Vec<String>,
    slot: Option<String>,
    max_era: Option<String>,
    owned: Option<std::collections::HashMap<String, u32>>,
    owned_tier: Option<std::collections::HashMap<String, u8>>,
) -> Vec<ItemDto> {
    gearplanner::list_items(
        &classes,
        slot.as_deref(),
        max_era.as_deref(),
        owned.as_ref(),
        owned_tier.as_ref(),
    )
}

/// why: "what if I upgrade this" preview, not a write to real ownership state

#[tauri::command]
pub fn get_item_at_tier(id: String, tier: u8) -> Option<ItemDto> {
    gearplanner::item_at_tier(&id, tier)
}

/// why: exaltation display re-derived with `exalts` socketed in

#[tauri::command]
pub fn get_item_with_exalts(
    id: String,
    tier: u8,
    exalts: std::collections::HashMap<String, String>,
) -> Option<ItemDto> {
    gearplanner::item_with_exalts(&id, tier, &exalts)
}

/// why: exaltation candidate list; `other_assignments` is every socket except this one

#[tauri::command]
pub fn get_exalt_candidates(
    id: String,
    socket_key: String,
    other_assignments: std::collections::HashMap<String, String>,
    classes: Vec<String>,
    max_era: Option<String>,
) -> Vec<ItemDto> {
    gearplanner::exalt_candidates(
        &id,
        &socket_key,
        &other_assignments,
        &classes,
        max_era.as_deref(),
    )
}

/// why: top candidates per slot, scored against classes/race; `level`
/// lets INT/WIS score as actual mana value not a flat fallback
#[tauri::command]
#[allow(clippy::too_many_arguments)] // why: each param is its own real, independently-optional filter
pub fn get_gear_recommendations(
    classes: Vec<String>,
    race: Option<String>,
    max_era: Option<String>,
    per_slot: Option<usize>,
    weights: Option<std::collections::HashMap<String, f64>>,
    level: Option<u8>,
    equipped: Option<std::collections::HashMap<String, String>>,
    owned: Option<std::collections::HashMap<String, u32>>,
    owned_tier: Option<std::collections::HashMap<String, u8>>,
) -> Vec<SlotRecommendationDto> {
    gearplanner::recommend(
        &classes,
        race.as_deref(),
        max_era.as_deref(),
        per_slot.unwrap_or(50),
        weights,
        level,
        equipped.as_ref(),
        owned.as_ref(),
        owned_tier.as_ref(),
    )
}

/// why: planner's "weights" panel -- makes the ranking explainable, not opaque

#[tauri::command]
pub fn get_gear_weights(
    classes: Vec<String>,
    level: Option<u8>,
) -> std::collections::HashMap<String, f64> {
    gearplanner::weights_for(&classes, level)
}

#[derive(Debug, Clone, Serialize)]
pub struct EraOptionsDto {
    /// why: ERA_ORDER oldest first; frontend adds a synthetic "All" on top
    pub eras: Vec<String>,
    pub current: String,
}

/// why: ERA_ORDER/CURRENT_ERA are Rust consts, not otherwise reachable over IPC

#[tauri::command]
pub fn get_era_options() -> EraOptionsDto {
    EraOptionsDto {
        eras: gearplanner::ERA_ORDER
            .iter()
            .map(|s| s.to_string())
            .collect(),
        current: gearplanner::CURRENT_ERA.to_string(),
    }
}

/// why: Settings module's volume/era preferences, separate file from `settings::NotificationSettings`

#[tauri::command]
pub fn get_preferences(app: AppHandle) -> Preferences {
    preferences::load(&app)
}

#[tauri::command]
pub fn set_preferences(app: AppHandle, mut prefs: Preferences) -> Result<Preferences, String> {
    // why: overlay_positions is backend-only -- never round-tripped
    // through PreferencesDto/the frontend at all (see its own doc), so
    // this call's own `prefs` never really carries it; whatever the
    // frontend's currentPrefs() happened to send for that field (its
    // own #[serde(default)] empty map, since the frontend doesn't know
    // the field exists) gets overwritten here with what's actually on
    // disk. Without this, changing something as unrelated as volume
    // would silently wipe every saved window position.
    prefs.overlay_positions = preferences::load(&app).overlay_positions;
    preferences::save(&app, &prefs)?;
    Ok(prefs)
}

/// why: Settings module's update-channel toggle -- see `updater`'s own doc
#[tauri::command]
pub async fn check_for_update(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<updater::UpdateInfoDto>, String> {
    updater::check_for_update(app, state).await
}

#[tauri::command]
pub async fn install_pending_update(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    updater::install_pending_update(app, state).await
}

/// why: feeds the Gear Planner's mana weighting; None mostly means "same
/// level the whole file", not "unknown"
#[tauri::command]
pub fn get_current_level(state: State<AppState>) -> Option<u8> {
    state.ingest.lock().unwrap().levels.latest()
}

/// why: reads and parses an inventory dump, matches equipped rows against the item catalog

#[tauri::command]
pub fn get_inventory_dump(
    state: State<AppState>,
    file: String,
) -> Result<InventoryDumpDto, String> {
    let base_dir = {
        let cfg = state.config.lock().unwrap();
        cfg.as_ref()
            .ok_or("no install folder configured yet")?
            .base_dir
            .clone()
    };
    let path = inventory::dump_path(&base_dir, &file).map_err(|e| e.to_string())?;
    let parsed = inventory::parse(&path).map_err(|e| e.to_string())?;
    let ing = state.ingest.lock().unwrap();
    Ok(gearplanner::resolve_inventory(
        &parsed,
        Some(&ing.exaltation_procs),
    ))
}

#[derive(Debug, Clone, Serialize)]
pub struct ExistingInventoryDumpDto {
    pub file: String,
    pub character: Option<String>,
}

/// why: init check for an existing dump from a past session, not just this run's live one

#[tauri::command]
pub fn find_existing_inventory_dump(state: State<AppState>) -> Option<ExistingInventoryDumpDto> {
    let base_dir = state.config.lock().unwrap().as_ref()?.base_dir.clone();
    let (file, character) = inventory::find_existing_dump(&base_dir)?;
    Some(ExistingInventoryDumpDto { file, character })
}

/// why: Maps module's pack picker; empty is valid, base game only

#[tauri::command]
pub fn list_map_packs(state: State<AppState>) -> Vec<String> {
    let Some(base_dir) = state
        .config
        .lock()
        .unwrap()
        .as_ref()
        .map(|c| c.base_dir.clone())
    else {
        return Vec::new();
    };
    mapsdata::list_map_packs(&base_dir)
}

/// why: Maps module's zone picker -- every zone with a map file

#[tauri::command]
pub fn list_map_zones(state: State<AppState>, pack: Option<String>) -> Vec<String> {
    let Some(base_dir) = state
        .config
        .lock()
        .unwrap()
        .as_ref()
        .map(|c| c.base_dir.clone())
    else {
        return Vec::new();
    };
    mapsdata::list_zone_names(&base_dir, pack.as_deref())
}

/// why: zone-first picker across every source -- avoids picking a pack before seeing if a map exists

#[tauri::command]
pub fn list_all_map_zones(state: State<AppState>) -> Vec<String> {
    let Some(base_dir) = state
        .config
        .lock()
        .unwrap()
        .as_ref()
        .map(|c| c.base_dir.clone())
    else {
        return Vec::new();
    };
    mapsdata::list_all_zone_names(&base_dir)
}

/// Which source(s) have a map for `zone` -- `null` for the base game, a
/// why: drives the "available versions" picker once a zone is chosen

#[tauri::command]
pub fn list_zone_versions(state: State<AppState>, zone: String) -> Vec<Option<String>> {
    let Some(base_dir) = state
        .config
        .lock()
        .unwrap()
        .as_ref()
        .map(|c| c.base_dir.clone())
    else {
        return Vec::new();
    };
    mapsdata::list_zone_versions(&base_dir, &zone)
}

/// why: Maps module's render data, merged from base file and every numbered sibling

#[tauri::command]
pub fn get_map_file(
    state: State<AppState>,
    pack: Option<String>,
    zone: String,
) -> Result<mapsdata::MapFileDto, String> {
    let base_dir = {
        let cfg = state.config.lock().unwrap();
        cfg.as_ref()
            .ok_or("no install folder configured yet")?
            .base_dir
            .clone()
    };
    let parsed =
        mapsdata::load_zone_map(&base_dir, pack.as_deref(), &zone).map_err(|e| e.to_string())?;
    Ok(parsed.into())
}

/// why: real walking route waypoints -- grid A* over wall geometry, Z-banded to the start floor

#[derive(Debug, Clone, Serialize)]
pub struct PathDto {
    pub waypoints: Vec<[f32; 3]>,
}

/// why: missing route is a real retryable outcome, not folded into an empty result

#[tauri::command]
pub fn find_walk_path(
    state: State<AppState>,
    pack: Option<String>,
    zone: String,
    from: [f32; 3],
    to: [f32; 3],
) -> Result<PathDto, String> {
    let base_dir = {
        let cfg = state.config.lock().unwrap();
        cfg.as_ref()
            .ok_or("no install folder configured yet")?
            .base_dir
            .clone()
    };
    let parsed =
        mapsdata::load_zone_map(&base_dir, pack.as_deref(), &zone).map_err(|e| e.to_string())?;
    let path = pathfind::find_path(&parsed, (from[0], from[1], from[2]), (to[0], to[1], to[2]))
        .ok_or("no walkable route found between those points")?;
    Ok(PathDto {
        waypoints: path.into_iter().map(|(x, y, z)| [x, y, z]).collect(),
    })
}

/// why: names its own spell so the player judges real access, not the backend assuming it

#[derive(Debug, Clone, Serialize)]
pub struct RouteHopDto {
    pub zone: String,
    pub kind: String,
    pub via_spell: Option<String>,
    pub distance: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ZoneRouteDto {
    pub hops: Vec<RouteHopDto>,
    pub total_distance: f64,
}

impl From<routing::ZoneRoute> for ZoneRouteDto {
    fn from(r: routing::ZoneRoute) -> Self {
        ZoneRouteDto {
            hops: r
                .hops
                .into_iter()
                .map(|h| {
                    let (kind, via_spell) = match h.kind {
                        routing::HopKind::Walk => ("walk".to_string(), None),
                        routing::HopKind::Teleport(spell) => ("teleport".to_string(), Some(spell)),
                        routing::HopKind::Succor => ("succor".to_string(), None),
                    };
                    RouteHopDto {
                        zone: h.zone,
                        kind,
                        via_spell,
                        distance: h.distance,
                    }
                })
                .collect(),
            total_distance: r.total_distance,
        }
    }
}

/// why: real distance-weighted route; teleports gated by the player's
/// assumed class/level (dominant confirmed configuration), walk-only
/// with no confirmed configuration yet. `known_start` is a real /loc
/// reading or teleport landing, zone-matched independently per source
/// since either can go stale in a different way -- returns the fresher.
fn live_start_position(
    ing: &crate::ingest::Ingest,
    base_dir: &std::path::Path,
    from_zone: &str,
) -> Option<(f32, f32, f32)> {
    // why: real reported bug -- /loc used to win unconditionally over a
    // later teleport/Origin confirmation for the same zone; now all
    // three sources compete on timestamp alone, freshest wins
    let mut best: Option<(eqlp_source::Millis, (f32, f32, f32))> = None;
    let mut consider = |ts: eqlp_source::Millis, pos: (f32, f32, f32)| {
        if best.is_none_or(|(best_ts, _)| ts > best_ts) {
            best = Some((ts, pos));
        }
    };

    if let Some((ts, x, y, z)) = ing.last_loc {
        if ing
            .zone
            .at(ts)
            .is_some_and(|raw| crate::zone::zone_matches(raw, from_zone))
        {
            consider(ts, (-y as f32, -x as f32, z as f32));
        }
    }
    if let Some((ts, landing)) = &ing.entered_via_teleport {
        if ing
            .zone
            .at(*ts)
            .is_some_and(|raw| crate::zone::zone_matches(raw, from_zone))
        {
            consider(
                *ts,
                (-landing.y as f32, -landing.x as f32, landing.z as f32),
            );
        }
    }
    // why: Origin's landing has no wiki-quoted coordinate, only a
    // confirmed zone -- `best_start_position`'s succor lookup stands in
    if let Some((ts, raw)) = &ing.learned_origin {
        if crate::zone::zone_matches(raw, from_zone) {
            consider(*ts, routing::best_start_position(base_dir, from_zone));
        }
    }
    best.map(|(_, pos)| pos)
}

#[tauri::command]
pub fn find_zone_route(
    app: AppHandle,
    state: State<AppState>,
    from_zone: String,
    to_zone: String,
) -> Result<ZoneRouteDto, String> {
    let base_dir = {
        let cfg = state.config.lock().unwrap();
        cfg.as_ref()
            .ok_or("no install folder configured yet")?
            .base_dir
            .clone()
    };
    let (player_classes, player_level, known_start) = {
        let ing = state.ingest.lock().unwrap();
        let dto = combat::class_configurations(&ing, "You");
        let (live_classes, level) = dto
            .configurations
            .first()
            .map(|c| {
                (
                    c.classes.clone(),
                    c.level_range.map(|(_, hi)| hi).unwrap_or(0),
                )
            })
            .unwrap_or_default();
        // why: level always comes from *this* live session, never the
        // saved profile -- level changes constantly and `profile.rs`
        // never stores it (see that module's own doc). Classes fall back
        // to the saved profile only when this session's own replay
        // hasn't confirmed a configuration for "You" yet at all -- once
        // it has, live evidence wins outright, full stop, regardless of
        // what's saved. See `preferences::Preferences::save_profile`'s
        // own doc for the whole policy this is one half of.
        let player_classes = if !live_classes.is_empty() {
            live_classes
        } else if preferences::load(&app).save_profile {
            state
                .status
                .lock()
                .unwrap()
                .character
                .as_deref()
                .and_then(|c| profile::for_character(&app, c))
                .map(|p| p.classes)
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        (
            player_classes,
            level,
            live_start_position(&ing, &base_dir, &from_zone),
        )
    };
    routing::find_zone_route(
        &base_dir,
        &from_zone,
        &to_zone,
        &player_classes,
        player_level,
        known_start,
    )
    .map(ZoneRouteDto::from)
    .ok_or_else(|| format!("no route found from {from_zone} to {to_zone}"))
}

#[derive(Debug, Clone, Serialize)]
pub struct LastLocationDto {
    pub ts_ms: eqlp_source::Millis,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    /// why: raw zone label at ts_ms, frontend matches loosely to avoid
    /// plotting a stale /loc on an unrelated map
    pub zone: Option<String>,
    /// why: resolved independently, not reused from current_map_zones --
    /// a /loc reading's zone can lag behind "right now"
    pub map_zones: Vec<String>,
}

/// why: "you are here" marker, rare -- a timestamped snapshot, not live tracking

#[tauri::command]
pub fn get_last_location(state: State<AppState>) -> Option<LastLocationDto> {
    let ing = state.ingest.lock().unwrap();
    let (ts_ms, x, y, z) = ing.last_loc?;
    let zone = ing.zone.at(ts_ms).map(str::to_string);
    let map_zones = map_zones_for_raw_label(zone.as_deref());
    Some(LastLocationDto {
        ts_ms,
        x,
        y,
        z,
        zone,
        map_zones,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct ZoneContextDto {
    /// why: raw zone label current right now
    pub current: Option<String>,
    /// why: label of the visit before this one, where the player likely walked in from
    pub previous: Option<String>,
    /// why: exact wiki-confirmed landing for this visit, if entered via
    /// teleport not an ordinary walk; frontend plots this over the weaker guess
    pub teleport_landing: Option<crate::teleportdata::TeleportLanding>,
    /// why: confirming timestamp; frontend compares against its own /loc
    /// timestamp, freshest wins -- same rule as `live_start_position`
    pub teleport_landing_ts: Option<eqlp_source::Millis>,
    /// why: real map-file shortnames, since internal shortnames bear no
    /// resemblance to display names; membership check, not a substring heuristic
    pub current_map_zones: Vec<String>,
}

/// why: fresh linear scan, not `Ingest`'s cache -- that cache is only
/// primed by combat, which a freshly-walked-into zone may never trigger.
/// Cheap enough at per-tick query rate. Empty is a real "don't know", not a guess.
fn map_zones_for_raw_label(raw: Option<&str>) -> Vec<String> {
    let Some(raw) = raw else { return Vec::new() };
    zonedata::zones()
        .iter()
        .find(|z| crate::zone::zone_matches(raw, &z.name))
        .and_then(|z| z.who_name.as_deref())
        .map(zonedata::map_shortnames)
        .unwrap_or_default()
}

/// why: Maps module's zone-identity + entrance-guess input; `current_map_zones`
/// confirms the open map is really current, `previous`/`teleport_landing` decide the guess
#[tauri::command]
pub fn get_zone_context(state: State<AppState>) -> ZoneContextDto {
    let ing = state.ingest.lock().unwrap();
    let ts = ing.now_ms();
    let current = ing.zone.at(ts).map(str::to_string);
    let current_map_zones = map_zones_for_raw_label(current.as_deref());
    // why: two independent confirmation sources compete on timestamp, same
    // "freshest wins" rule applies for routing. No base_dir yet is a
    // real "can't compute" for the Origin side specifically, falls through.
    let wiki_landing = ing.entered_via_teleport.clone();
    let origin_landing = (|| {
        let (origin_ts, raw) = ing.learned_origin.as_ref()?;
        if !crate::zone::zone_matches(raw, current.as_deref()?) {
            return None;
        }
        let base_dir = state.config.lock().unwrap().as_ref()?.base_dir.clone();
        let (x, y, z) = routing::best_start_position(&base_dir, current.as_deref()?);
        Some((
            *origin_ts,
            crate::teleportdata::TeleportLanding {
                class: crate::teleportdata::TeleportClass::Any,
                x: x as f64,
                y: y as f64,
                z: z as f64,
                zone: current.clone().unwrap_or_default(),
                level: 1,
            },
        ))
    })();
    let (teleport_landing, teleport_landing_ts) = match (wiki_landing, origin_landing) {
        (Some((wts, _)), Some((ots, ol))) if ots > wts => (Some(ol), Some(ots)),
        (Some((wts, wl)), _) => (Some(wl), Some(wts)),
        (None, Some((ots, ol))) => (Some(ol), Some(ots)),
        (None, None) => (None, None),
    };
    ZoneContextDto {
        current,
        previous: ing.zone.label_before(ts).map(str::to_string),
        teleport_landing,
        teleport_landing_ts,
        current_map_zones,
    }
}

/// why: NPC-overlay candidate list, toggle-able not auto-applied -- can't
/// reliably resolve wiki names against internal EQ shortcodes
#[tauri::command]
pub fn list_npc_zone_candidates(map_zone_name: String) -> Vec<String> {
    npcdata::candidate_zones(&map_zone_name)
}

#[derive(Debug, Clone, Serialize)]
pub struct NpcMarkerDto {
    pub name: String,
    pub x: f32,
    pub y: f32,
    /// why: None for a 2D-only wiki spot, most real entries
    pub z: Option<f32>,
}

/// why: real spawn points for an exact `Npc::zone` value, not a name to fuzzy-match again

#[tauri::command]
pub fn get_npc_markers_for_zone(zone: String) -> Vec<NpcMarkerDto> {
    npcdata::markers_for_zone(&zone)
        .into_iter()
        .map(|(name, x, y, z)| NpcMarkerDto { name, x, y, z })
        .collect()
}

/// why: kind + label pairs, so a new kind shows up automatically once added

#[derive(Debug, Clone, Serialize)]
pub struct NotificationKindDto {
    pub kind: String,
    pub label: String,
}

#[tauri::command]
pub fn list_notification_kinds() -> Vec<NotificationKindDto> {
    notifications::ALL_KINDS
        .iter()
        .map(|&kind| NotificationKindDto {
            kind: kind.to_string(),
            label: notifications::kind_label(kind).to_string(),
        })
        .collect()
}

/// why: direct pass-through, NotificationSettings already derives Serialize

#[tauri::command]
pub fn get_notification_settings(app: AppHandle) -> settings::NotificationSettings {
    settings::load(&app)
}

#[tauri::command]
pub fn set_notification_enabled(
    app: AppHandle,
    kind: String,
    on: bool,
) -> Result<settings::NotificationSettings, String> {
    let mut s = settings::load(&app);
    s.set_enabled(&kind, on);
    settings::save(&app, &s)?;
    Ok(s)
}

/// why: picks and copies a sound file in, saves as `kind`'s custom sound;
/// Ok(None) on cancel, not an error, same stance as `pick_log_directory`
#[tauri::command]
pub async fn pick_notification_sound(
    app: AppHandle,
    kind: String,
) -> Result<Option<settings::NotificationSettings>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Choose a notification sound")
        .add_filter("Audio", &["mp3", "wav", "ogg", "m4a"])
        .pick_file(move |file| {
            let _ = tx.send(file);
        });
    let Some(file) = rx.await.ok().flatten() else {
        return Ok(None); // user cancelled
    };
    let path = file.into_path().map_err(|e| e.to_string())?;
    let filename = settings::store_custom_sound(&app, &kind, &path)?;
    let mut s = settings::load(&app);
    s.set_custom_sound(&kind, Some(filename));
    settings::save(&app, &s)?;
    Ok(Some(s))
}

/// why: reverts to the frontend's synthesized default -- deletes the stored file and clears

#[tauri::command]
pub fn clear_notification_sound(
    app: AppHandle,
    kind: String,
) -> Result<settings::NotificationSettings, String> {
    let mut s = settings::load(&app);
    if let Some(filename) = s.custom_sound(&kind) {
        settings::delete_custom_sound(&app, filename);
    }
    s.set_custom_sound(&kind, None);
    settings::save(&app, &s)?;
    Ok(s)
}

/// why: ready for `new Audio(url)`; None falls back to the synthesized default, not an error

#[tauri::command]
pub fn get_notification_sound_data(app: AppHandle, kind: String) -> Option<String> {
    let s = settings::load(&app);
    settings::custom_sound_data_url(&app, &kind, &s)
}

/// why: Spellbook builder's file picker -- every real UI config file in the base folder

#[tauri::command]
pub fn list_ui_files(state: State<AppState>) -> Result<Vec<uifiles::UiFileInfoDto>, String> {
    let base_dir = state
        .config
        .lock()
        .unwrap()
        .as_ref()
        .ok_or("no install folder configured yet")?
        .base_dir
        .clone();
    Ok(uifiles::list_ui_files(&base_dir))
}

/// why: one UI file's real content, read-only

#[tauri::command]
pub fn get_ui_file(
    state: State<AppState>,
    file: String,
) -> Result<uifiles::ParsedUiFileDto, String> {
    let base_dir = state
        .config
        .lock()
        .unwrap()
        .as_ref()
        .ok_or("no install folder configured yet")?
        .base_dir
        .clone();
    let path = uifiles::ui_file_path(&base_dir, &file).map_err(|e| e.to_string())?;
    uifiles::parse_ini(&path).map_err(|e| e.to_string())
}

/// why: a real character's saved [SpellLoadouts] -- see spellbookfiles's
/// own doc. `file` is one of list_ui_files's own "hotbuttons"-kind
/// entries (the non-`UI_`-prefixed one, which is where loadouts live).
#[tauri::command]
pub fn load_spellbook_file(
    state: State<AppState>,
    file: String,
) -> Result<spellbookfiles::SpellbookFileDto, String> {
    let base_dir = state
        .config
        .lock()
        .unwrap()
        .as_ref()
        .ok_or("no install folder configured yet")?
        .base_dir
        .clone();
    spellbookfiles::load_spellbook(&base_dir, &file)
}

#[tauri::command]
pub fn save_spellbook_file(
    state: State<AppState>,
    file: String,
    loadouts: Vec<spellbookfiles::SpellLoadoutDto>,
) -> Result<(), String> {
    let base_dir = state
        .config
        .lock()
        .unwrap()
        .as_ref()
        .ok_or("no install folder configured yet")?
        .base_dir
        .clone();
    spellbookfiles::save_spellbook(&base_dir, &file, &loadouts)
}

/// why: real "save as" -- forks `source_file`'s pair (hotbuttons + its
/// `UI_` layout counterpart) under a new `<Character>_<Zone>` stem,
/// never touching the source. See spellbookfiles::save_spellbook_as.
#[tauri::command]
pub fn save_spellbook_file_as(
    state: State<AppState>,
    source_file: String,
    new_stem: String,
    loadouts: Vec<spellbookfiles::SpellLoadoutDto>,
) -> Result<String, String> {
    let base_dir = state
        .config
        .lock()
        .unwrap()
        .as_ref()
        .ok_or("no install folder configured yet")?
        .base_dir
        .clone();
    spellbookfiles::save_spellbook_as(&base_dir, &source_file, &new_stem, &loadouts)
}

/// why: resolves a batch of catalog spell names to their real numeric
/// ids in one call -- None per entry means spells_us.txt has no
/// exact-name entry, not that the frontend did anything wrong. Batched
/// on purpose: filling a loadout's worth of empty slots used to call
/// this once per slot, each a fresh full spells_us.txt reparse -- real,
/// measured slowdown. See spellbookfiles::resolve_spell_ids.
#[tauri::command]
pub fn resolve_spellbook_spell_ids(
    state: State<AppState>,
    names: Vec<String>,
) -> Result<Vec<Option<i64>>, String> {
    let base_dir = state
        .config
        .lock()
        .unwrap()
        .as_ref()
        .ok_or("no install folder configured yet")?
        .base_dir
        .clone();
    Ok(spellbookfiles::resolve_spell_ids(&base_dir, &names))
}

#[cfg(test)]
mod live_start_position_tests {
    use super::*;
    use std::path::Path;

    /// why: real reported bug -- /loc used to win unconditionally over a later teleport confirmation
    #[test]
    fn a_later_teleport_confirmation_wins_over_an_earlier_loc_reading() {
        let mut ing = crate::ingest::Ingest::default();
        ing.zone.enter(1_000, "Oggok".to_string());
        ing.last_loc = Some((1_000, 100.0, 200.0, 5.0));
        // why: a later, fresher confirmation for the same zone
        ing.zone.enter(2_000, "Oggok".to_string());
        ing.entered_via_teleport = Some((
            2_000,
            crate::teleportdata::TeleportLanding {
                class: crate::teleportdata::TeleportClass::Wizard,
                x: 300.0,
                y: 400.0,
                z: 10.0,
                zone: "Oggok".to_string(),
                level: 1,
            },
        ));
        let pos = live_start_position(&ing, Path::new("/nonexistent"), "Oggok");
        // /loc-space -> map-file transform: (-y, -x, z).
        assert_eq!(
            pos,
            Some((-400.0, -300.0, 10.0)),
            "the fresher teleport landing should win, not the earlier /loc"
        );
    }

    /// why: reverse must hold -- a genuinely fresher /loc beats a stale teleport confirmation
    #[test]
    fn a_later_loc_reading_wins_over_an_earlier_teleport_confirmation() {
        let mut ing = crate::ingest::Ingest::default();
        ing.zone.enter(1_000, "Oggok".to_string());
        ing.entered_via_teleport = Some((
            1_000,
            crate::teleportdata::TeleportLanding {
                class: crate::teleportdata::TeleportClass::Wizard,
                x: 300.0,
                y: 400.0,
                z: 10.0,
                zone: "Oggok".to_string(),
                level: 1,
            },
        ));
        ing.last_loc = Some((2_000, 100.0, 200.0, 5.0));
        let pos = live_start_position(&ing, Path::new("/nonexistent"), "Oggok");
        assert_eq!(
            pos,
            Some((-200.0, -100.0, 5.0)),
            "the fresher /loc reading should win, not the earlier teleport landing"
        );
    }
}
