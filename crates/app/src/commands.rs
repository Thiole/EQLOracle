//! The IPC surface. Kept intentionally small: pick a folder, commit to it,
//! ask what's currently true. Everything live-updating goes over the
//! `parse-tick` / `parse-error` events emitted from `tail_worker`, not
//! request/response.

use crate::config::{self, AppConfig};
use crate::state::AppState;
use crate::tail_worker::{self, Snapshot};
use serde::Serialize;
use std::path::PathBuf;
use tauri::{AppHandle, State};

#[derive(Debug, Clone, Serialize)]
pub struct StatusDto {
    pub configured: bool,
    pub snapshot: Snapshot,
}

#[tauri::command]
pub fn get_status(state: State<AppState>) -> StatusDto {
    StatusDto {
        configured: state.config.lock().unwrap().is_some(),
        snapshot: state.snapshot.lock().unwrap().clone(),
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
    let handle = tail_worker::spawn(app.clone(), dir, state.snapshot.clone());
    *state.worker.lock().unwrap() = Some(handle);

    Ok(StatusDto { configured: true, snapshot: state.snapshot.lock().unwrap().clone() })
}
