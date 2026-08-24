//! why: in-app update check/install -- no redirect to GitHub to grab the
//! newest release by hand. Endpoint is built fresh per check from the
//! user's own saved channel preference (see `preferences::UpdateChannel`),
//! not the static default in tauri.conf.json (that's only the Public
//! fallback for the rare caller that skips this and uses the plugin directly).

use crate::preferences::{self, UpdateChannel};
use crate::state::AppState;
use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_updater::UpdaterExt;

fn endpoint_for(channel: UpdateChannel) -> &'static str {
    match channel {
        UpdateChannel::Public => {
            "https://github.com/Thiole/EQLOracle/releases/download/latest/latest.json"
        }
        UpdateChannel::Beta => {
            "https://github.com/Thiole/EQLOracle/releases/download/testing/latest.json"
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfoDto {
    pub version: String,
    pub current_version: String,
    /// why: release body text, whatever the channel's own release notes say
    pub notes: Option<String>,
}

/// why: checks whichever channel is currently saved, stores the found
/// Update (if any) in AppState for install_pending_update to consume --
/// a real network round trip, so a real Result, not silently swallowed.
/// Not `#[tauri::command]` itself -- see commands.rs's own thin wrapper,
/// same convention every other command in this app follows.
pub async fn check_for_update(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<UpdateInfoDto>, String> {
    let channel = preferences::load(&app).update_channel;
    let url = endpoint_for(channel)
        .parse()
        .map_err(|e: url::ParseError| e.to_string())?;
    let updater = app
        .updater_builder()
        .endpoints(vec![url])
        .map_err(|e| e.to_string())?
        .build()
        .map_err(|e| e.to_string())?;
    let found = updater.check().await.map_err(|e| e.to_string())?;

    let dto = found.as_ref().map(|u| UpdateInfoDto {
        version: u.version.clone(),
        current_version: u.current_version.clone(),
        notes: u.body.clone(),
    });
    *state.pending_update.lock().unwrap() = found;
    Ok(dto)
}

/// why: consumes whatever check_for_update last found -- an explicit
/// error (not a silent no-op) if the frontend calls this without a real
/// check first, or the check found nothing to install
pub async fn install_pending_update(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let update = state.pending_update.lock().unwrap().clone();
    let Some(update) = update else {
        return Err("no pending update -- call check_for_update first".to_string());
    };
    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|e| e.to_string())?;
    app.restart();
}
