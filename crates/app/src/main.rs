//! Desktop shell entry point.
//!
//! v1 scope: a docked window, a first-launch folder picker, and a live feed
//! of parsed lines from whichever `eqlog_*.txt` the game is currently
//! writing. No overlay -- see `FOUNDATION.md` #4, window role is a
//! negotiated capability added later, not assumed here.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod combat;
mod commands;
mod config;
mod ingest;
mod parser;
mod state;
mod tail_worker;

use state::AppState;
use tauri::Manager;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::new())
        .setup(|app| {
            let handle = app.handle().clone();
            let state = app.state::<AppState>();
            if let Some(cfg) = config::load(&handle) {
                if cfg.log_dir.is_dir() {
                    let dir = cfg.log_dir.clone();
                    *state.config.lock().unwrap() = Some(cfg);
                    let worker = tail_worker::spawn(handle, dir, state.ingest.clone(), state.status.clone());
                    *state.worker.lock().unwrap() = Some(worker);
                }
                // Directory on record but gone (drive unmounted, prefix
                // moved): fall through to the setup screen rather than
                // spin up a worker that can only ever see `Missing`.
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_status,
            commands::pick_log_directory,
            commands::set_log_directory,
            commands::list_zone_visits,
            commands::list_encounters,
            commands::get_combat_summary,
        ])
        .run(tauri::generate_context!())
        .expect("error while running eqlp-app");
}
