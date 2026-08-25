//! Desktop shell entry point.
//!
//! v1 scope: a docked window, a first-launch folder picker, and a live feed
//! of parsed lines from whichever `eqlog_*.txt` the game is currently
//! writing. No overlay -- see `FOUNDATION.md` #4, window role is a
//! negotiated capability added later, not assumed here.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// why: modules live in lib.rs -- a bin-side `mod ingest;` here would
// silently produce a second, incompatible `Ingest` type
use eqlp_app::{commands, config, history, state::AppState, tail_worker};
use tauri::Manager;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(AppState::new())
        .setup(|app| {
            let handle = app.handle().clone();
            let state = app.state::<AppState>();
            // why: every start parses clean, history doesn't survive a restart
            history::reset(&handle);
            if let Some(cfg) = config::load(&handle) {
                let log_dir = cfg.log_dir();
                if log_dir.is_dir() {
                    *state.config.lock().unwrap() = Some(cfg);
                    let worker = tail_worker::spawn(
                        handle,
                        log_dir,
                        state.ingest.clone(),
                        state.status.clone(),
                    );
                    *state.worker.lock().unwrap() = Some(worker);
                }
                // why: dir on record but gone -- fall through to setup screen
            }
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
            commands::get_default_gear_classes,
            commands::list_gear_items,
            commands::get_item_at_tier,
            commands::get_item_with_exalts,
            commands::get_exalt_candidates,
            commands::get_gear_recommendations,
            commands::get_gear_weights,
            commands::get_inventory_dump,
            commands::find_existing_inventory_dump,
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
            commands::get_character_estimate,
            commands::get_session,
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running eqlp-app");
}
