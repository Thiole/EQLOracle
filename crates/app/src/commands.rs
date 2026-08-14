//! The IPC surface.
//!
//! Two shapes: `get_status` / `pick_log_directory` / `set_log_directory` for
//! the toolbar and first-launch setup, and the Combat module's read-only
//! queries (`list_zone_visits`, `list_encounters`, `get_combat_summary`),
//! which run straight against the shared `Ingest` -- the parsed db -- with
//! no reparsing. Everything live-updating besides that goes over the
//! `parse-tick` / `parse-error` events emitted from `tail_worker`.

use crate::combat::{self, CombatSummaryDto, EncounterDto, ZoneVisitDto};
use crate::config::{self, AppConfig};
use crate::ingest::LineCounts;
use crate::state::AppState;
use crate::tail_worker::{self, TailStatus};
use serde::Serialize;
use std::path::PathBuf;
use tauri::{AppHandle, State};

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

/// Opens the native folder picker. Returns `None` if the user cancels --
/// that is not an error, it just means nothing changes.
///
/// Uses the plugin's async callback API, not `blocking_pick_folder`.
/// Blocking a command thread on the dialog result ties this to whatever
/// thread that command happened to run on, and on Linux the dialog goes
/// through GTK's main loop / xdg-desktop-portal -- a context blocking
/// doesn't reliably mesh with. The callback form is the one path the
/// plugin runs through the right thread on every platform; we just await
/// it instead of blocking for it.
#[tauri::command]
pub async fn pick_log_directory(app: AppHandle) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Select your EverQuest Legends Logs folder")
        .pick_folder(move |folder| {
            let _ = tx.send(folder);
        });
    rx.await.ok().flatten().map(|p| p.to_string())
}

/// Commits to a directory: persists it, then (re)starts the tail worker.
/// Called both from first-launch setup and from "change folder" later, so
/// switching a running app to a new directory is not a special case.
#[tauri::command]
pub fn set_log_directory(app: AppHandle, state: State<AppState>, path: String) -> Result<StatusDto, String> {
    let dir = PathBuf::from(&path);
    if !dir.is_dir() {
        return Err(format!("{path} is not a directory"));
    }

    let cfg = AppConfig { log_dir: dir.clone() };
    config::save(&app, &cfg)?;
    *state.config.lock().unwrap() = Some(cfg);

    if let Some(old) = state.worker.lock().unwrap().take() {
        old.stop();
    }
    let handle = tail_worker::spawn(app.clone(), dir, state.ingest.clone(), state.status.clone());
    *state.worker.lock().unwrap() = Some(handle);

    Ok(StatusDto {
        configured: true,
        status: state.status.lock().unwrap().clone(),
        counts: state.ingest.lock().unwrap().counts.clone(),
    })
}

/// Every zone visit seen so far, newest first, with how many fights each
/// holds. The Combat module's first dropdown.
#[tauri::command]
pub fn list_zone_visits(state: State<AppState>) -> Vec<ZoneVisitDto> {
    combat::list_zone_visits(&state.ingest.lock().unwrap())
}

/// Every encounter, optionally narrowed to one zone visit, newest first.
/// The Combat module's second dropdown. `zone_visit` is `None` for no
/// filter, `-1` for the "Unknown" (pre-first-zone-line) bucket, otherwise a
/// visit index -- see `combat::matches_visit`.
#[tauri::command]
pub fn list_encounters(state: State<AppState>, zone_visit: Option<i64>) -> Vec<EncounterDto> {
    combat::list_encounters(&state.ingest.lock().unwrap(), zone_visit)
}

/// The Combat module's main panel: one encounter if `encounter_id` is
/// given, else every encounter in `zone_visit`, else everything parsed.
#[tauri::command]
pub fn get_combat_summary(state: State<AppState>, zone_visit: Option<i64>, encounter_id: Option<u32>) -> CombatSummaryDto {
    combat::summarize(&state.ingest.lock().unwrap(), zone_visit, encounter_id)
}
