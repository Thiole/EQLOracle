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
use crate::craftlog::{self, CraftLogEntryDto};
use crate::debugview::{self, DebugEncounterDto, GameStateDto, UnmatchedCoverageDto};
use crate::dpscalc::{self, DamageSpellDto};
use crate::emumaps;
use crate::gearplanner::{self, InventoryDumpDto, ItemDto, SlotRecommendationDto};
use crate::history::{self, ParseRecord};
use crate::ingest::LineCounts;
use crate::inventory;
use crate::itemdata;
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
use crate::state::{AppState, LockRecover};
use crate::tail_worker::{self, TailStatus};
use crate::tradeskilldata::{self, TradeskillSkill};
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
        configured: state.config.lock_recover().is_some(),
        status: state.status.lock_recover().clone(),
        counts: state.ingest.lock_recover().counts.clone(),
    }
}

/// why: native folder picker, None on cancel (not an error). Async
/// callback API, not blocking_pick_folder -- Linux's GTK/portal dialog
/// doesn't reliably mesh with a blocked command thread.
#[tauri::command]
pub async fn pick_log_directory(app: AppHandle) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    let mut dialog = app
        .dialog()
        .file()
        .set_title("Select your EverQuest Legends install folder");
    // why: parent to the main window -- an unparented Windows dialog can
    // open BEHIND the app and never get focus, which reads as "the
    // button does nothing" (real Windows first-launch report); the
    // FirstLaunch paste-a-path fallback covers whatever this doesn't
    if let Some(win) = app.get_webview_window("main") {
        dialog = dialog.set_parent(&win);
    }
    dialog.pick_folder(move |folder| {
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
    // why: validate AND repair before persisting -- picking `Logs`
    // itself resolves to its parent ("figure it out for them", the
    // player's ask), and an unrecognizable folder errors instead of
    // saving a config that wedges the next launch (real Windows report)
    let dir = config::normalize_base_dir(&dir)?;

    let cfg = AppConfig { base_dir: dir };
    config::save(&app, &cfg)?;
    let log_dir = cfg.log_dir();
    *state.config.lock_recover() = Some(cfg);

    if let Some(old) = state.worker.lock_recover().take() {
        old.stop();
    }
    let handle = tail_worker::spawn(
        app.clone(),
        log_dir,
        state.ingest.clone(),
        state.status.clone(),
    );
    *state.worker.lock_recover() = Some(handle);

    Ok(StatusDto {
        configured: true,
        status: state.status.lock_recover().clone(),
        counts: state.ingest.lock_recover().counts.clone(),
    })
}

/// why: Combat module's first dropdown -- zone visits, newest first, fight counts

#[tauri::command]
pub fn list_zone_visits(state: State<AppState>) -> Vec<ZoneVisitDto> {
    combat::list_zone_visits(&state.ingest.lock_recover())
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
        &state.ingest.lock_recover(),
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
    combat::list_zone_encounters(&state.ingest.lock_recover(), &zone_id, limit.unwrap_or(30))
}

/// why: damage/drops fetched separately so the initial list never waits on them

#[tauri::command]
pub fn get_encounter_detail(
    state: State<AppState>,
    encounter_id: u32,
) -> Option<EncounterDetailDto> {
    combat::encounter_detail(&state.ingest.lock_recover(), encounter_id)
}

/// why: NPC page's kills/pulls totals plus recent fights

#[tauri::command]
pub fn get_mob_stats(state: State<AppState>, mob_name: String) -> MobStatsDto {
    monsters::mob_stats(&state.ingest.lock_recover(), &mob_name)
}

#[tauri::command]
pub fn list_mob_encounters(
    state: State<AppState>,
    mob_name: String,
    limit: Option<usize>,
) -> Vec<ZoneEncounterDto> {
    combat::list_mob_encounters(&state.ingest.lock_recover(), &mob_name, limit.unwrap_or(30))
}

/// why: Debug module's table -- recent encounters with raw and resolved zone tags

#[tauri::command]
pub fn list_debug_encounters(
    state: State<AppState>,
    limit: Option<usize>,
) -> Vec<DebugEncounterDto> {
    debugview::list_debug_encounters(&state.ingest.lock_recover(), limit.unwrap_or(100))
}

/// why: Debug module's "Unparsed" tab -- unmatched shapes ranked by count

#[tauri::command]
pub fn get_unmatched_coverage(state: State<AppState>, top: Option<usize>) -> UnmatchedCoverageDto {
    debugview::unmatched_coverage(&state.ingest.lock_recover(), top.unwrap_or(100))
}

/// why: Debug module's "Game State" tab -- compact live dump of current
/// party/class/level belief, not a polished feature

#[tauri::command]
pub fn get_game_state(state: State<AppState>) -> GameStateDto {
    debugview::game_state(&state.ingest.lock_recover())
}

/// why: Combat module's primary view -- allies sorted by total damage descending

#[tauri::command]
pub fn list_allies(
    state: State<AppState>,
    zone_visit: Option<i64>,
    encounter_id: Option<u32>,
    confirmed_only: Option<bool>,
) -> Vec<AllyDto> {
    combat::list_allies(
        &state.ingest.lock_recover(),
        zone_visit,
        encounter_id,
        confirmed_only.unwrap_or(false),
    )
}

/// why: Combat module's drill-down -- one ally's breakdown, or the whole selection's

#[tauri::command]
pub fn get_combat_summary(
    state: State<AppState>,
    zone_visit: Option<i64>,
    encounter_id: Option<u32>,
    actor: Option<String>,
    confirmed_only: Option<bool>,
) -> CombatSummaryDto {
    combat::summarize(
        &state.ingest.lock_recover(),
        zone_visit,
        encounter_id,
        actor.as_deref(),
        confirmed_only.unwrap_or(false),
    )
}

/// Per-entity damage-over-time bars for one fight's scrub bar.
#[tauri::command]
pub fn get_fight_timeline(state: State<AppState>, encounter_id: u32) -> Option<FightTimelineDto> {
    combat::fight_timeline(&state.ingest.lock_recover(), encounter_id)
}

/// What clicking a point on the scrub bar shows: every entity's state and a
/// snapshot DPS reading as of that instant.
#[tauri::command]
pub fn get_fight_state_at(
    state: State<AppState>,
    encounter_id: u32,
    ts_ms: i64,
) -> Vec<EntityStateDto> {
    combat::fight_state_at(&state.ingest.lock_recover(), encounter_id, ts_ms)
}

/// why: every configuration for one entity, most zone visits first; empty if nothing confirmed yet

#[tauri::command]
pub fn get_class_configurations(state: State<AppState>, name: String) -> ClassConfigurationsDto {
    combat::class_configurations(&state.ingest.lock_recover(), &name)
}

/// why: Endgame's Raiding tab, curated list with confirmed kills/tiers/loot

#[tauri::command]
pub fn get_raids(state: State<AppState>) -> Vec<RaidRowDto> {
    raiding::list_raid_rows(&state.ingest.lock_recover())
}

/// why: "Sky - Primary Class Unlocks" tab -- final reward items only, not raw materials

#[tauri::command]
pub fn get_sky_class_unlocks(state: State<AppState>) -> Vec<skyquests::SkyClassUnlockDto> {
    let base_dir = state
        .config
        .lock_recover()
        .as_ref()
        .map(|c| c.base_dir.clone());
    skyquests::list_class_unlocks(&state.ingest.lock_recover(), base_dir.as_deref())
}

/// why: "Sky - Quests" tab -- every material turn-in, full detail

#[tauri::command]
pub fn get_sky_quests(state: State<AppState>) -> Vec<skyquests::SkyClassDto> {
    let base_dir = state
        .config
        .lock_recover()
        .as_ref()
        .map(|c| c.base_dir.clone());
    skyquests::list_quests(&state.ingest.lock_recover(), base_dir.as_deref())
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
        &state.ingest.lock_recover(),
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
    let ing = state.ingest.lock_recover();
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
    let ing = state.ingest.lock_recover();
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
    overview::session(&state.ingest.lock_recover())
}

/// why: Overview Session card's own "restart" button -- see
/// Ingest::reset_session's own doc for why this isn't persisted

#[tauri::command]
pub fn reset_session(state: State<AppState>) -> SessionDto {
    let mut ing = state.ingest.lock_recover();
    ing.reset_session();
    overview::session(&ing)
}

/// why: Game Data's own top-of-page disclaimer -- what catalog this is
/// and how stale it might be, not silently presented as live
#[derive(Debug, Clone, serde::Serialize)]
pub struct GameDataMetaDto {
    pub source: String,
    /// why: None if the scrape itself never recorded one (older pack) --
    /// shown as "unknown" in the UI, never a guessed date
    pub scraped: Option<String>,
}

#[tauri::command]
pub fn get_game_data_meta() -> GameDataMetaDto {
    GameDataMetaDto {
        source: "https://eqlwiki.com".to_string(),
        scraped: itemdata::scraped().map(str::to_string),
    }
}

/// why: static recipe catalog for the Tradeskill module -- every core
/// tradeskill's own recipe list, baked in at compile time, no Ingest needed
#[tauri::command]
pub fn get_tradeskill_catalog() -> Vec<TradeskillSkill> {
    tradeskilldata::skills().to_vec()
}

/// why: real craft attempts this file has ever recorded, joined against
/// the static catalog above -- see craftlog's own doc
#[tauri::command]
pub fn get_craft_log(state: State<AppState>) -> Vec<CraftLogEntryDto> {
    craftlog::craft_log(&state.ingest.lock_recover())
}

/// why: every AA purchase this session plus total spent; no UI consumes this yet

#[tauri::command]
pub fn get_aa_log(state: State<AppState>) -> AaLogDto {
    progression::aa_log(&state.ingest.lock_recover())
}

/// why: Character module's Spellbook subpage -- known spells enriched with catalog stats

#[tauri::command]
pub fn get_spellbook(state: State<AppState>) -> Vec<SpellbookEntryDto> {
    progression::spellbook(&state.ingest.lock_recover())
}

/// why: Spellbook builder's picker -- shows an already-ranked spell's real rank

#[tauri::command]
pub fn get_spell_ranks(state: State<AppState>) -> HashMap<String, u8> {
    progression::spell_ranks(&state.ingest.lock_recover())
}

/// why: every damage-capable spell, rank-adjusted, unfiltered -- caller applies its own filtering

#[tauri::command]
pub fn get_damage_spells(state: State<AppState>, assume_max_rank: bool) -> Vec<DamageSpellDto> {
    dpscalc::list_damage_spells(&state.ingest.lock_recover(), assume_max_rank)
}

/// why: Loot History module's one view -- mob types, kills, loot

#[tauri::command]
pub fn list_mobs(state: State<AppState>) -> Vec<MobDto> {
    monsters::list_mobs(&state.ingest.lock_recover())
}

/// why: Social tab's Guild sub-channel

#[tauri::command]
pub fn get_guild_chat(state: State<AppState>) -> Vec<ChatMessageDto> {
    chat::guild_chat(&state.ingest.lock_recover())
}

/// why: Social tab's Party sub-channel

#[tauri::command]
pub fn get_party_chat(state: State<AppState>) -> Vec<ChatMessageDto> {
    chat::party_chat(&state.ingest.lock_recover())
}

/// why: Social tab's Raid sub-channel

#[tauri::command]
pub fn get_raid_chat(state: State<AppState>) -> Vec<ChatMessageDto> {
    chat::raid_chat(&state.ingest.lock_recover())
}

/// why: Social tab's PM player list, most-recent-message first

#[tauri::command]
pub fn list_pm_threads(state: State<AppState>) -> Vec<PmThreadDto> {
    chat::pm_threads(&state.ingest.lock_recover())
}

/// why: one PM thread's whole history, oldest first

#[tauri::command]
pub fn get_pm_history(state: State<AppState>, player: String) -> Vec<ChatMessageDto> {
    chat::pm_history(&state.ingest.lock_recover(), &player)
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
    combat::live_meter(&state.ingest.lock_recover())
}

/// why: overlay's timed-effects widget -- same polled-on-tick shape as
/// get_live_meter, see effects.rs's own doc
#[tauri::command]
pub fn get_status_effects(state: State<AppState>) -> crate::effects::StatusEffectsDto {
    crate::effects::status_effects(&state.ingest.lock_recover())
}

/// why: Skill Tracker widget's own-cooldowns section -- see skilltracker.rs's own doc
#[tauri::command]
pub fn get_skill_status(state: State<AppState>) -> Vec<crate::skilltracker::SkillStatusDto> {
    crate::skilltracker::skill_status(&state.ingest.lock_recover())
}

/// why: Drop Watch widget -- see dropwatch.rs's own doc. Unfiltered
/// (every currently-relevant mob's full known drop list); the frontend
/// intersects against tracked_drop_items, same split get_skill_status uses.
#[tauri::command]
pub fn get_drop_watch(state: State<AppState>) -> Vec<crate::dropwatch::DropWatchRowDto> {
    crate::dropwatch::drop_watch(&state.ingest.lock_recover())
}

/// why: "why did I just die" -- see deathrecap.rs's own doc. `death_ts`
/// picks a specific death from the returned list; None means the most
/// recent. Both halves in one call: the recap plus every death
/// timestamp this session, so the frontend's picker never needs a
/// second round trip.
#[tauri::command]
pub fn get_death_recap(
    state: State<AppState>,
    death_ts: Option<i64>,
) -> (Option<crate::deathrecap::DeathRecapDto>, Vec<i64>) {
    let ing = state.ingest.lock_recover();
    (
        crate::deathrecap::recap(&ing, death_ts),
        crate::deathrecap::death_timestamps(&ing),
    )
}

/// why: Drop Watch's "you just got one, remove it?" prompt -- see
/// TrackedLootDto's own doc. `items` is whatever the frontend is
/// currently tracking, one call covers all of them.
#[tauri::command]
pub fn get_tracked_loot_status(
    state: State<AppState>,
    items: Vec<String>,
) -> Vec<crate::dropwatch::TrackedLootDto> {
    crate::dropwatch::loot_status(&mut state.ingest.lock_recover(), &items)
}

/// why: Skill Tracker widget's target-effects section -- see targeteffects.rs's own doc
#[tauri::command]
pub fn get_target_effects(state: State<AppState>) -> crate::targeteffects::TargetEffectsDto {
    crate::targeteffects::target_effects(&state.ingest.lock_recover())
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

/// why: CC Tracker's own layout knob -- "small"/"medium"/"large" mapped
/// to a logical-pixel (width, height) just big enough for 3 squares at
/// that size plus the shared panel chrome (CCTrackerWidget's own p-2,
/// OverlayApp's own p-2 wrapper). Mirrored exactly on the frontend by
/// ccSize.ts's own CC_SIZE_WINDOW_DIMS -- the two must stay in sync by
/// hand (no shared codegen across the Rust/TS boundary here), which is
/// why both sides comment-reference each other. An unrecognized string
/// (old/downgraded install, hand-edited prefs file) falls back to
/// "small" rather than erroring -- same contract as `theme`.
fn cc_tracker_dims(size: &str) -> (f64, f64) {
    match size {
        "medium" => (250.0, 60.0),
        "large" => (280.0, 76.0),
        _ => (220.0, 48.0),
    }
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
    //
    // why per-widget default size: most widgets are real data tables
    // (rows of players, cooldowns, drops) and want the room. CC Tracker
    // is three fixed squares at a user-chosen size -- see
    // cc_tracker_dims's own doc.
    let (w, h) = match widget.as_str() {
        "cc_tracker" => cc_tracker_dims(&preferences::load(&app).overlay_cc_tracker_size),
        _ => (360.0, 240.0),
    };
    // why: built hidden, shown only after hide_from_window_switcher --
    // the GTK type hint must be set before the window first maps for
    // the window manager to honor it at manage time
    let mut builder =
        WebviewWindowBuilder::new(&app, &label, WebviewUrl::App("overlay.html".into()))
            .title(format!("EQL Oracle Overlay -- {widget}"))
            .inner_size(w, h)
            .transparent(true)
            .decorations(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .visible(false)
            .shadow(false);
    // why: restore a widget's last saved position -- see
    // preferences::OverlayPosition's doc (captured in set_overlay_locked
    // below); absent until then, opens at the OS default position. Only
    // restored when the point still lands on a CURRENT monitor -- a
    // position saved against an unplugged second display or an old
    // resolution otherwise opens the window fully off-screen, which
    // reads as "overlay enabled but not showing at all" (the likely
    // shape of the Windows report; same failure exists everywhere).
    if let Some(pos) = preferences::load(&app).overlay_positions.get(&widget) {
        if position_on_some_monitor(&app, pos.x, pos.y) {
            builder = builder.position(pos.x, pos.y);
        }
    }
    let window = builder.build().map_err(|e| e.to_string())?;
    // why: one main-thread closure, strict internal order -- three real
    // constraints at once, and the ordering between tao's request
    // channel and run_on_main_thread's own queue is not guaranteed
    // across separate calls:
    // (1) the GTK type hint is a direct GTK call, main-thread only, and
    //     must land before the window first maps;
    // (2) show() must come BEFORE set_ignore_cursor_events -- real
    //     crash, caught live: tao's CursorIgnoreEvents handler unwraps
    //     the GdkWindow, which is None until the window is realized, so
    //     click-through against the still-hidden window panicked the
    //     main thread and took the whole app down ("app crashes when
    //     enabling overlay");
    // (3) both queued requests (show, then ignore) drain FIFO after
    //     this closure returns, so ignore always runs against a
    //     realized window.
    let w = window.clone();
    let click_through = cap.capability == WindowCapability::ClickThrough;
    let _ = window.run_on_main_thread(move || {
        hide_from_window_switcher(&w);
        let _ = w.show();
        // why: ClickThrough only -- Floating alone (never actually
        // reachable today, detect() only ever returns Docked or
        // ClickThrough, kept as its own tier for when finer Wayland
        // detection becomes possible) would still block clicks on the
        // game underneath it
        if click_through {
            let _ = w.set_ignore_cursor_events(true);
            ensure_layered_still_renders(&w);
        }
    });
    Ok(())
}

/// why: a saved LOGICAL position is only worth restoring if it still
/// lands on a live monitor -- checked against each monitor's own
/// logical rect (physical geometry / its scale factor), with a small
/// margin so a window whose top-left sits a few px past an edge (a
/// drag that hugged the border) still counts. Monitor enumeration
/// failing falls back to "don't restore", never "restore blind".
fn position_on_some_monitor(app: &AppHandle, x: f64, y: f64) -> bool {
    const MARGIN: f64 = 32.0;
    let Ok(monitors) = app.available_monitors() else {
        return false;
    };
    monitors.iter().any(|m| {
        let scale = m.scale_factor();
        let pos = m.position().to_logical::<f64>(scale);
        let size = m.size().to_logical::<f64>(scale);
        x >= pos.x - MARGIN
            && x <= pos.x + size.width + MARGIN
            && y >= pos.y - MARGIN
            && y <= pos.y + size.height + MARGIN
    })
}

/// why: tao's Windows click-through adds WS_EX_LAYERED but never calls
/// SetLayeredWindowAttributes -- and MSDN is explicit that a layered
/// window "will not become visible until SetLayeredWindowAttributes or
/// UpdateLayeredWindow has been called". So enabling an overlay on
/// Windows produced a running, permanently INVISIBLE window ("maybe
/// its not showing at all" -- the report, and the player's own read:
/// "what if its just how the overlay is set to work on windows
/// incorrectly"). Full alpha keeps the window rendered; the webview's
/// own per-pixel transparency is DWM composition, unaffected by the
/// layered alpha. Winit fixes the same gap the same way. No-op off
/// Windows.
fn ensure_layered_still_renders(window: &tauri::WebviewWindow) {
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetWindowLongPtrW, SetLayeredWindowAttributes, SetWindowLongPtrW, GWL_EXSTYLE,
            LWA_ALPHA, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TRANSPARENT,
        };
        if let Ok(hwnd) = window.hwnd() {
            let hwnd = hwnd.0 as isize;
            unsafe {
                // why: self-asserted, not trusted to tao's posted flag
                // application -- an overlay opens at the OS default
                // position, ON TOP of the main window, and a
                // click-through that silently failed to stick leaves an
                // invisible topmost window eating every click under it
                // ("cant click anything in the app", the Windows
                // report). TRANSPARENT passes clicks through,
                // LAYERED+alpha keeps it rendering (tao sets the styles
                // but never the attributes), NOACTIVATE keeps it from
                // ever taking focus -- the standard overlay triple.
                let ex = GetWindowLongPtrW(hwnd as _, GWL_EXSTYLE);
                SetWindowLongPtrW(
                    hwnd as _,
                    GWL_EXSTYLE,
                    ex | (WS_EX_TRANSPARENT | WS_EX_LAYERED | WS_EX_NOACTIVATE) as isize,
                );
                SetLayeredWindowAttributes(hwnd as _, 0, 255, LWA_ALPHA);
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    let _ = window;
}

/// why: skip_taskbar alone covers the TASKBAR, not the alt-tab
/// switcher -- KWin's switcher filters on window TYPE (Utility and
/// friends are excluded, skip-taskbar windows are not), and tao's
/// Windows skip_taskbar only calls ITaskbarList::DeleteTab, which
/// likewise leaves alt-tab untouched. Four always-on-top widget
/// windows cycling through alt-tab as separate "apps" is exactly what
/// an overlay must not do. Best-effort on both platforms: a failure
/// leaves the widget working, just visible in the switcher again.
/// Called while the window is still hidden (builder sets
/// visible(false)) so the WM sees the hint at first map.
fn hide_from_window_switcher(window: &tauri::WebviewWindow) {
    #[cfg(target_os = "linux")]
    {
        use gtk::prelude::GtkWindowExt;
        if let Ok(gtk_window) = window.gtk_window() {
            gtk_window.set_type_hint(gtk::gdk::WindowTypeHint::Utility);
        }
    }
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
        };
        if let Ok(hwnd) = window.hwnd() {
            // why: double cast -- tauri's `windows` crate HWND has been
            // isize in some versions and *mut c_void in others; through
            // isize it lands on windows-sys's own alias either way
            let hwnd = hwnd.0 as isize;
            unsafe {
                let ex = GetWindowLongPtrW(hwnd as _, GWL_EXSTYLE);
                SetWindowLongPtrW(
                    hwnd as _,
                    GWL_EXSTYLE,
                    (ex | WS_EX_TOOLWINDOW as isize) & !(WS_EX_APPWINDOW as isize),
                );
            }
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    let _ = window;
}

/// why: live-pushes to this widget's own open window -- a no-op, not an
/// error, when it isn't open; persistence is the caller's own
/// setPreferences call.
///
/// emit_to alone does NOT scope this the way its own doc implies --
/// real bug, caught live: with every overlay window covered by the same
/// "overlay-*" capability glob (capabilities/default.json), emit_to's
/// permission check treats the whole glob as one audience and delivers
/// to every window matching it, not just the one whose label was
/// passed in. Confirmed with temporary two-sided logging: the SEND side
/// always carried the right label, but every open overlay-* window's
/// own listener fired regardless. So every payload here now carries the
/// target widget too, and each window filters to its own identity
/// (currentOverlayWidget()) before acting -- correct regardless of
/// whatever emit_to does under the hood, not dependent on understanding
/// its exact scoping behavior for this capability shape.
#[tauri::command]
pub fn set_overlay_opacity(app: AppHandle, widget: String, opacity: f64) {
    let label = overlay_label(&widget);
    if app.get_webview_window(&label).is_some() {
        let _ = app.emit_to(&label, "overlay-opacity", (widget, opacity.clamp(0.0, 1.0)));
    }
}

/// why: the SEPARATE "everything" fade (see
/// preferences::default_overall_opacity's doc) -- same live-push/
/// persistence split as set_overlay_opacity, its own event name so the
/// overlay window can tell the two apart. Same widget-in-payload
/// filtering as set_overlay_opacity -- see its own doc.
#[tauri::command]
pub fn set_overlay_overall_opacity(app: AppHandle, widget: String, opacity: f64) {
    let label = overlay_label(&widget);
    if app.get_webview_window(&label).is_some() {
        let _ = app.emit_to(
            &label,
            "overlay-overall-opacity",
            (widget, opacity.clamp(0.0, 1.0)),
        );
    }
}

/// why: same live-push/persist split as set_overlay_opacity, but resizes
/// the real OS window instead of a CSS value -- this command only
/// forwards the raw string, it never touches actual pixel dimensions.
/// The window receiving it is the one that already knows its own new
/// size (OverlayApp.svelte's own 'overlay-size' listener, via ccSize.
/// ts's own table) and resizes itself; only CC Tracker uses this today,
/// shaped generically (a `widget` param, same as every other overlay
/// setting) so the next widget with a size preset doesn't need a new
/// command. Same widget-in-payload filtering as set_overlay_opacity --
/// see its own doc.
#[tauri::command]
pub fn set_overlay_size(app: AppHandle, widget: String, size: String) {
    let label = overlay_label(&widget);
    if app.get_webview_window(&label).is_some() {
        let _ = app.emit_to(&label, "overlay-size", (widget, size));
    }
}

/// why: "where did that window go" -- a click-through, semi-transparent,
/// always-on-top widget is easy to lose track of, especially right
/// after a reposition or a display change. Brings it to front (a
/// click-through window never gets real focus/raise from clicking
/// through it) and emits an event carrying the target widget;
/// OverlayApp.svelte owns the actual flash effect and filters to its
/// own identity -- see set_overlay_opacity's own doc on why the payload
/// carries the widget now instead of trusting emit_to alone. No-op if
/// the widget's window isn't open.
#[tauri::command]
pub fn locate_overlay(app: AppHandle, widget: String) {
    let label = overlay_label(&widget);
    if let Some(w) = app.get_webview_window(&label) {
        let _ = w.set_focus();
        let _ = app.emit_to(&label, "overlay-locate", widget);
    }
}

/// why: click-through (locked, default) blocks dragging since every
/// click passes to the game underneath. Unlocking turns real decorations
/// back on so a title-bar drag works (XWayland/KWin: the borderless
/// drag-region trick doesn't move the window there). No-op if the
/// widget's window isn't open, or this session never had click-through.
///
/// Also saves the widget's new position (preferences::OverlayPosition)
/// at the moment of re-locking, not continuously mid-drag -- a nudge
/// that's never re-locked shouldn't half-persist. Logical pixels:
/// outer_position() returns physical pixels, converted via the window's
/// scale factor for HiDPI correctness.

#[tauri::command]
pub fn set_overlay_locked(app: AppHandle, widget: String, locked: bool) -> Result<(), String> {
    if windowcap::detect().capability != WindowCapability::ClickThrough {
        return Ok(());
    }
    if let Some(w) = app.get_webview_window(&overlay_label(&widget)) {
        w.set_ignore_cursor_events(locked)
            .map_err(|e| e.to_string())?;
        if locked {
            // why: this command runs OFF the event-loop thread, so the
            // style change above was POSTED, not applied -- the attribute
            // call must queue behind it (same PostMessage FIFO) or it
            // lands on a not-yet-layered window as a no-op. The enable
            // path's call sits inside run_on_main_thread already, where
            // tao executes inline and ordering is direct.
            let w2 = w.clone();
            let _ = w.run_on_main_thread(move || ensure_layered_still_renders(&w2));
        }
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

/// why: the frontend can't cfg(target_os) -- whether the main window
/// runs frameless with an in-app title bar is a backend platform fact.
/// Windows only: a custom title bar was tried everywhere and reverted
/// on Linux -- KWin/XWayland silently drops the drag-region move
/// request on an undecorated window (see Toolbar.svelte's own doc), so
/// Linux keeps native decorations.
#[derive(Serialize)]
pub struct UiShellDto {
    pub custom_titlebar: bool,
}

#[tauri::command]
pub fn get_ui_shell() -> UiShellDto {
    UiShellDto {
        custom_titlebar: cfg!(target_os = "windows"),
    }
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
    monsters::item_loot_history(&state.ingest.lock_recover(), &item)
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
    gearplanner::default_classes(&state.ingest.lock_recover(), &name)
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
    let on_disk = preferences::load(&app);
    prefs.overlay_positions = on_disk.overlay_positions;
    // why: same backend-only stance -- planner state has its own
    // commands (get/set_planner_state); an unrelated Settings save must
    // not wipe a hand-set race/level
    prefs.planner_race = on_disk.planner_race;
    prefs.planner_levels = on_disk.planner_levels;
    preferences::save(&app, &prefs)?;
    Ok(prefs)
}

/// why: Character Planner persistence -- hand-set race plus ONLY the
/// levels the user typed over the estimate (presence = "user updated",
/// the flag the planner shows). Its own pair of commands, not part of
/// PreferencesDto, so Settings round trips can never clobber it -- see
/// set_preferences' own doc.
#[derive(Serialize)]
pub struct PlannerStateDto {
    pub race: Option<String>,
    pub levels: HashMap<String, u8>,
}

#[tauri::command]
pub fn get_planner_state(app: AppHandle) -> PlannerStateDto {
    let p = preferences::load(&app);
    PlannerStateDto {
        race: p.planner_race,
        levels: p.planner_levels,
    }
}

/// why: whole-state write -- the frontend owns the merge (it knows which
/// edit happened); race None clears, empty levels clears (the "Estimate
/// levels" reset path).
#[tauri::command]
pub fn set_planner_state(
    app: AppHandle,
    race: Option<String>,
    levels: HashMap<String, u8>,
) -> Result<(), String> {
    let mut prefs = preferences::load(&app);
    prefs.planner_race = race;
    prefs.planner_levels = levels;
    preferences::save(&app, &prefs)
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
pub fn restart_app(app: AppHandle) {
    updater::restart_app(app)
}

#[tauri::command]
pub async fn install_pending_update(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    updater::install_pending_update(app, state).await
}

/// why: Info page's own version display -- same source
/// check_for_update's current_version reads, exposed standalone so
/// showing it doesn't need a network round trip. Backend command through
/// invoke(), not a raw @tauri-apps/api call (see invoke.ts's doc).
#[tauri::command]
pub fn get_app_version(app: AppHandle) -> String {
    app.package_info().version.to_string()
}

/// why: feeds the Gear Planner's mana weighting; None mostly means "same
/// level the whole file", not "unknown"
#[tauri::command]
pub fn get_current_level(state: State<AppState>) -> Option<u8> {
    state.ingest.lock_recover().levels.latest()
}

/// why: reads and parses an inventory dump, matches equipped rows against the item catalog

#[tauri::command]
pub fn get_inventory_dump(
    state: State<AppState>,
    file: String,
) -> Result<InventoryDumpDto, String> {
    let base_dir = {
        let cfg = state.config.lock_recover();
        cfg.as_ref()
            .ok_or("no install folder configured yet")?
            .base_dir
            .clone()
    };
    let path = inventory::dump_path(&base_dir, &file).map_err(|e| e.to_string())?;
    let parsed = inventory::parse(&path).map_err(|e| e.to_string())?;
    let ing = state.ingest.lock_recover();
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
    let base_dir = state.config.lock_recover().as_ref()?.base_dir.clone();
    let (file, character) = inventory::find_existing_dump(&base_dir)?;
    Some(ExistingInventoryDumpDto { file, character })
}

/// why: shared by `locate_item` and `get_inventory_browser` -- both want
/// "the latest dump on disk, parsed", neither cares which file. None
/// covers every reason that can fail (no base folder configured, no
/// dump yet, a read/parse error) -- "unknown", same stance every other
/// inventory-derived read already takes, not worth distinguishing for a
/// read-only view.
fn latest_parsed_inventory(state: &State<AppState>) -> Option<inventory::ParsedInventory> {
    let base_dir = state.config.lock_recover().as_ref()?.base_dir.clone();
    let (file, _character) = inventory::find_existing_dump(&base_dir)?;
    let path = inventory::dump_path(&base_dir, &file).ok()?;
    inventory::parse(&path).ok()
}

/// why: "where is my X" -- GdLink's own locate affordance, wherever an
/// item name already renders in the app.
#[tauri::command]
pub fn locate_item(state: State<AppState>, name: String) -> Vec<inventory::InventoryLocation> {
    latest_parsed_inventory(&state)
        .map(|parsed| parsed.locate(&name).to_vec())
        .unwrap_or_default()
}

/// why: Character module's Inventory tab -- bags/bank/depot/key ring,
/// grouped by real container. Equip-doll slots deliberately excluded,
/// see `inventory::ParsedInventory::containers`' own doc.
#[tauri::command]
pub fn get_inventory_browser(state: State<AppState>) -> Vec<inventory::InventoryContainerDto> {
    latest_parsed_inventory(&state)
        .map(|parsed| parsed.containers)
        .unwrap_or_default()
}

/// why: Maps module's pack picker; empty is valid, base game only

#[tauri::command]
pub fn list_map_packs(state: State<AppState>) -> Vec<String> {
    let Some(base_dir) = state
        .config
        .lock_recover()
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
        .lock_recover()
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
        .lock_recover()
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
        .lock_recover()
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
        let cfg = state.config.lock_recover();
        cfg.as_ref()
            .ok_or("no install folder configured yet")?
            .base_dir
            .clone()
    };
    let parsed =
        mapsdata::load_zone_map(&base_dir, pack.as_deref(), &zone).map_err(|e| e.to_string())?;
    Ok(parsed.into())
}

/// why: real walking route waypoints. `source` says which engine
/// produced them: "navmesh" (EQEmu Detour mesh -- true walkable
/// surfaces, multi-floor correct; see emumaps.rs) or "lines" (the
/// original grid A* over the game map's wall geometry, the fallback
/// when a zone's mesh isn't cached yet).

#[derive(Debug, Clone, Serialize)]
pub struct PathDto {
    pub waypoints: Vec<[f32; 3]>,
    pub source: &'static str,
}

/// why: missing route is a real retryable outcome, not folded into an empty result

#[tauri::command]
pub fn find_walk_path(
    app: AppHandle,
    state: State<AppState>,
    pack: Option<String>,
    zone: String,
    from: [f32; 3],
    to: [f32; 3],
) -> Result<PathDto, String> {
    // why: navmesh first -- the mesh knows floors/ramps/water the line
    // maps can't; disk-cache-only (ensure_emu_zone owns the download)
    if let Ok(app_data) = app.path().app_data_dir() {
        if let Some(nav) = emumaps::load_nav(&app_data, &zone) {
            if let Some(waypoints) = nav.find_path(from, to) {
                return Ok(PathDto {
                    waypoints,
                    source: "navmesh",
                });
            }
            // why: fall through -- endpoints off the mesh (a bad click
            // in the void) can still resolve on the line grid
        }
    }
    let base_dir = {
        let cfg = state.config.lock_recover();
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
        source: "lines",
    })
}

/// why: fetches a zone's EQEmu nav+collision files into the app-data
/// cache -- fired by the Maps view when a zone opens, so the
/// pathfinding/best-Z call sites never block on network. Returns which
/// halves are now available.

#[derive(Debug, Clone, Serialize)]
pub struct EmuZoneStatusDto {
    pub nav: bool,
    pub geo: bool,
}

#[tauri::command]
pub async fn ensure_emu_zone(app: AppHandle, zone: String) -> EmuZoneStatusDto {
    let Ok(app_data) = app.path().app_data_dir() else {
        return EmuZoneStatusDto {
            nav: false,
            geo: false,
        };
    };
    let (nav, geo) = emumaps::ensure_zone(&app_data, &zone).await;
    EmuZoneStatusDto { nav, geo }
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
        let cfg = state.config.lock_recover();
        cfg.as_ref()
            .ok_or("no install folder configured yet")?
            .base_dir
            .clone()
    };
    let (player_classes, player_level, known_start) = {
        let ing = state.ingest.lock_recover();
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
                .lock_recover()
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
pub fn get_last_location(app: AppHandle, state: State<AppState>) -> Option<LastLocationDto> {
    let ing = state.ingest.lock_recover();
    let (ts_ms, x, y, z) = ing.last_loc?;
    let zone = ing.zone.at(ts_ms).map(str::to_string);
    let map_zones = map_zones_for_raw_label(zone.as_deref());
    drop(ing);
    // why: best-Z snap -- /loc's own z is the character's origin (often
    // mid-model, mid-jump, or on a mount) and the map view picks its
    // floor level from it; the collision mesh's nearest surface is the
    // real ground. Only when the zone's geo is cached (see emumaps.rs);
    // raw z stands otherwise.
    let mut ground_z = None;
    if let Ok(app_data) = app.path().app_data_dir() {
        for shortname in &map_zones {
            if let Some(geo) = emumaps::load_geo(&app_data, shortname) {
                ground_z = geo.best_z(x as f32, y as f32, z as f32).map(|g| g as f64);
                if ground_z.is_some() {
                    break;
                }
            }
        }
    }
    Some(LastLocationDto {
        ts_ms,
        x,
        y,
        z: ground_z.unwrap_or(z),
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
    let ing = state.ingest.lock_recover();
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
        let base_dir = state.config.lock_recover().as_ref()?.base_dir.clone();
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

#[derive(Debug, Clone, Serialize)]
pub struct NpcNavPointDto {
    /// why: raw wiki zone -- what the map's own npc-overlay bridge matches on
    pub zone: String,
    /// why: the zonedata name `find_zone_route` routes on -- exact hit
    /// first, alias-folded second; None means unroutable (walk path in
    /// the zone itself still works)
    pub route_zone: Option<String>,
    pub x: f32,
    pub y: f32,
    pub z: Option<f32>,
}

/// why: "set a path to this NPC, same way pick-destination does" -- the
/// player's own ask. Coordinates come from the wiki spawn points the
/// npc-overlay markers already plot; zone resolution reuses the same
/// alias fold `zone_matches` applies everywhere else.
#[tauri::command]
pub fn get_npc_nav_points(name: String) -> Vec<NpcNavPointDto> {
    npcdata::nav_points_for(&name)
        .into_iter()
        .map(|(zone, x, y, z)| {
            let route_zone = crate::zonedata::zones()
                .iter()
                .find(|zd| zd.name == zone || crate::zone::zone_matches(&zone, &zd.name))
                .map(|zd| zd.name.clone());
            NpcNavPointDto {
                zone,
                route_zone,
                x,
                y,
                z,
            }
        })
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
    let mut dialog = app
        .dialog()
        .file()
        .set_title("Choose a notification sound")
        .add_filter("Audio", &["mp3", "wav", "ogg", "m4a"]);
    // why: same Windows behind-the-window failure pick_log_directory
    // already parents against -- unparented, this reads as "the button
    // does nothing"
    if let Some(win) = app.get_webview_window("main") {
        dialog = dialog.set_parent(&win);
    }
    dialog.pick_file(move |file| {
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
        .lock_recover()
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
        .lock_recover()
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
        .lock_recover()
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
        .lock_recover()
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
        .lock_recover()
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
        .lock_recover()
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

#[cfg(test)]
mod cc_tracker_dims_tests {
    use super::*;

    #[test]
    fn each_named_preset_gets_its_own_real_size() {
        assert_eq!(cc_tracker_dims("small"), (220.0, 48.0));
        assert_eq!(cc_tracker_dims("medium"), (250.0, 60.0));
        assert_eq!(cc_tracker_dims("large"), (280.0, 76.0));
    }

    /// why: an old/downgraded install or a hand-edited prefs file must
    /// still open a window, not panic or return zero-size garbage
    #[test]
    fn an_unrecognized_size_falls_back_to_small() {
        assert_eq!(cc_tracker_dims("huge"), cc_tracker_dims("small"));
        assert_eq!(cc_tracker_dims(""), cc_tracker_dims("small"));
    }

    /// why: real data both ways -- "Plane of Sky" resolves to a routable
    /// zonedata name for find_zone_route; the raw wiki zone survives
    /// untouched for the npc-overlay bridge either way
    #[test]
    fn npc_nav_points_resolve_a_routable_zone_where_one_exists() {
        let pts = get_npc_nav_points("Eye of Veeshan".into());
        assert_eq!(pts.len(), 1);
        assert_eq!(pts[0].zone, "Plane of Sky");
        assert!(
            pts[0].route_zone.is_some(),
            "Plane of Sky must resolve to a routable zonedata name"
        );
        let zintrin = get_npc_nav_points("Guard Zintrin".into());
        assert_eq!(zintrin.len(), 2);
        assert_eq!(zintrin[0].zone, "East Freeport");
    }
}
