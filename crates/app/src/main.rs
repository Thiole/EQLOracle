//! Desktop shell entry point.
//!
//! v1 scope: a docked window, a first-launch folder picker, and a live feed
//! of parsed lines from whichever `eqlog_*.txt` the game is currently
//! writing. The overlay (`FOUNDATION.md` #4's own negotiated window-role
//! capability) came later, gated by `windowcap::detect`, never assumed.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// why: modules live in lib.rs -- a bin-side `mod ingest;` here would
// silently produce a second, incompatible `Ingest` type
use eqlp_app::{
    commands, config, history,
    state::{AppState, LockRecover},
    tail_worker, updater,
};
use tauri::Manager;

/// why: GTK reads this env var lazily, at its own (first-use) init time --
/// setting it here, before `tauri::Builder` ever touches GTK, is enough,
/// no shell wrapper needed. "x11,wayland" prefers X11 -- on a native X11
/// desktop that's a no-op; on Wayland it routes this app's own window
/// through XWayland (GNOME/KDE both ship it by default), the
/// near-universal X11-compat layer, so `always_on_top`/click-through
/// (native Wayland can't do either, tao#1134) actually work. Falls back
/// to "wayland" itself only if XWayland genuinely isn't there --
/// windowcap.rs's own DISPLAY check decides whether that fallback
/// happened, not a guess. Linux-only: the env var means nothing
/// elsewhere, but scoped anyway to keep the "why" attached to where it matters.
#[cfg(target_os = "linux")]
fn prefer_x11_backend() {
    // why: doesn't override a value the user (or a launcher) already set --
    // someone who explicitly forced GDK_BACKEND=wayland meant that
    if std::env::var("GDK_BACKEND").is_err() {
        std::env::set_var("GDK_BACKEND", "x11,wayland");
    }
}

fn main() {
    // why: generated up front (pure data, touches nothing) -- selfinstall
    // needs the context's version, the one CI's --config override sets on
    // testing builds; CARGO_PKG_VERSION never sees that override
    let context = tauri::generate_context!();
    // why: before anything else -- may exec into the installed copy and
    // never return; nothing (GTK included) should have initialized yet
    #[cfg(target_os = "linux")]
    eqlp_app::selfinstall::install_or_handoff(
        context
            .config()
            .version
            .as_deref()
            .unwrap_or(env!("CARGO_PKG_VERSION")),
    );
    #[cfg(target_os = "linux")]
    prefer_x11_backend();

    tauri::Builder::default()
        // why: registered FIRST, upstream's own requirement, so the
        // duplicate process exits before any of its own state spins up.
        // A second launch focuses the running instance's main window
        // instead of starting a second app -- see Cargo.toml's own doc
        // on what a real duplicate breaks.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(AppState::new())
        // why: caught live -- Tauri's own default is "keep the process
        // alive as long as any window exists", which on this app means
        // closing the main window alone leaves the whole process (and
        // the overlay, if it's open) running invisibly in the
        // background forever. The overlay is an auxiliary window of the
        // main one, never the reverse -- closing main always ends the
        // whole app, the overlay's own close is just its own close.
        .on_window_event(|window, event| {
            if window.label() == "main"
                && matches!(event, tauri::WindowEvent::CloseRequested { .. })
            {
                // why: clean exit -- clears the unclean-exit sentinel so
                // the next launch keeps its webview cache; a killed run
                // never reaches this, see updater::mark_clean_exit
                updater::mark_clean_exit(window.app_handle());
                window.app_handle().exit(0);
            }
        })
        .setup(|app| {
            let handle = app.handle().clone();
            let state = app.state::<AppState>();
            // why: before anything else, and before this process's own
            // window/webview exist at all -- see updater::
            // clear_stale_webview_cache_if_needed's own doc for why this
            // specific timing (not during the OLD process's own restart,
            // a real, twice-reported bug) is what actually makes it safe
            updater::clear_stale_webview_cache_if_needed(&handle);
            // why: every start parses clean, history doesn't survive a restart
            history::reset(&handle);
            if let Some(cfg) = config::load(&handle) {
                let log_dir = cfg.log_dir();
                if log_dir.is_dir() {
                    *state.config.lock_recover() = Some(cfg);
                    let worker = tail_worker::spawn(
                        handle,
                        log_dir,
                        state.ingest.clone(),
                        state.status.clone(),
                    );
                    *state.worker.lock_recover() = Some(worker);
                }
                // why: dir on record but gone -- fall through to setup screen
            }
            // why: NOT auto-reopened from a saved "was it on" flag --
            // overlay_enabled isn't a persisted preference (see
            // preferences.rs's own doc), same "never trust stale carried-
            // over state" stance save_profile takes for class detection.
            // Every launch starts with the overlay off; opacity/widget
            // choices still carry over once it's turned back on this session.
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_status,
            commands::pick_log_directory,
            commands::set_log_directory,
            commands::list_zone_visits,
            commands::list_encounters,
            commands::list_allies,
            commands::get_combat_summary,
            commands::get_fight_timeline,
            commands::get_fight_state_at,
            commands::get_class_configurations,
            commands::get_configuration_zone_visits,
            commands::get_raids,
            commands::get_sky_class_unlocks,
            commands::get_sky_quests,
            commands::list_ui_files,
            commands::get_ui_file,
            commands::load_spellbook_file,
            commands::save_spellbook_file,
            commands::save_spellbook_file_as,
            commands::resolve_spellbook_spell_ids,
            commands::get_mob_history,
            commands::get_loadout_summary,
            commands::list_mobs,
            commands::get_window_capability,
            commands::get_live_meter,
            commands::get_status_effects,
            commands::get_skill_status,
            commands::get_target_effects,
            commands::get_drop_watch,
            commands::get_death_recap,
            commands::get_tracked_loot_status,
            commands::set_overlay_enabled,
            commands::set_overlay_opacity,
            commands::set_overlay_overall_opacity,
            commands::set_overlay_size,
            commands::locate_overlay,
            commands::set_overlay_locked,
            commands::get_guild_chat,
            commands::get_party_chat,
            commands::get_raid_chat,
            commands::list_pm_threads,
            commands::get_pm_history,
            commands::get_default_gear_classes,
            commands::list_gear_items,
            commands::get_item_at_tier,
            commands::get_item_with_exalts,
            commands::get_exalt_candidates,
            commands::get_gear_recommendations,
            commands::get_gear_weights,
            commands::get_inventory_dump,
            commands::find_existing_inventory_dump,
            commands::locate_item,
            commands::get_inventory_browser,
            commands::list_map_packs,
            commands::list_map_zones,
            commands::list_all_map_zones,
            commands::list_zone_versions,
            commands::get_map_file,
            commands::find_walk_path,
            commands::find_zone_route,
            commands::get_last_location,
            commands::get_zone_context,
            commands::list_npc_zone_candidates,
            commands::get_npc_markers_for_zone,
            commands::get_npc_nav_points,
            commands::get_current_level,
            commands::list_zones,
            commands::list_npcs,
            commands::get_mob_aliases,
            commands::get_item_loot_history,
            commands::list_zone_encounters,
            commands::get_encounter_detail,
            commands::get_mob_stats,
            commands::list_mob_encounters,
            commands::list_debug_encounters,
            commands::get_unmatched_coverage,
            commands::get_game_state,
            commands::get_character_estimate,
            commands::get_session,
            commands::reset_session,
            commands::get_game_data_meta,
            commands::get_tradeskill_catalog,
            commands::get_craft_log,
            commands::get_aa_log,
            commands::get_spellbook,
            commands::get_spell_ranks,
            commands::get_damage_spells,
            commands::list_spells,
            commands::list_spell_effects,
            commands::get_spell_stacking_groups,
            commands::list_aa,
            commands::list_notification_kinds,
            commands::get_notification_settings,
            commands::set_notification_enabled,
            commands::pick_notification_sound,
            commands::clear_notification_sound,
            commands::get_notification_sound_data,
            commands::get_era_options,
            commands::get_preferences,
            commands::set_preferences,
            commands::check_for_update,
            commands::install_pending_update,
            commands::restart_app,
            commands::get_app_version,
        ])
        .run(context)
        .expect("error while running eqlp-app");
}
