//! why: in-app update check/install -- no redirect to GitHub to grab the
//! newest release by hand. Endpoint is built fresh per check from the
//! user's own saved channel preference (see `preferences::UpdateChannel`),
//! not the static default in tauri.conf.json (that's only the Public
//! fallback for the rare caller that skips this and uses the plugin directly).

use crate::preferences::{self, UpdateChannel};
use crate::state::AppState;
use serde::Serialize;
#[cfg(target_os = "linux")]
use tauri::Manager;
use tauri::{AppHandle, State};
use tauri_plugin_updater::UpdaterExt;

/// why: same channel-to-tag mapping the release workflow itself uses
/// (see 3-release.yml's own "Determine channel" step) -- one real
/// source for it here too, endpoint_for/release_url_for both just
/// format this instead of repeating the match
fn tag_for(channel: UpdateChannel) -> &'static str {
    match channel {
        UpdateChannel::Public => "latest",
        UpdateChannel::Beta => "testing",
    }
}

fn endpoint_for(channel: UpdateChannel) -> String {
    format!(
        "https://github.com/Thiole/EQLOracle/releases/download/{}/latest.json",
        tag_for(channel)
    )
}

/// why: Spencer's own ask -- the update prompt should link to the real
/// GitHub release/changelog, not just show the notes text inline with
/// nowhere to actually go read more
fn release_url_for(channel: UpdateChannel) -> String {
    format!(
        "https://github.com/Thiole/EQLOracle/releases/tag/{}",
        tag_for(channel)
    )
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfoDto {
    pub version: String,
    pub current_version: String,
    /// why: release body text, whatever the channel's own release notes say
    pub notes: Option<String>,
    /// why: Spencer's own ask -- a real link to the GitHub release page,
    /// not just the notes text with nowhere to click through to
    pub release_url: String,
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
        release_url: release_url_for(channel),
    });
    *state.pending_update.lock().unwrap() = found;
    Ok(dto)
}

/// why: real bug, caught live -- an install-and-restart froze on a
/// blank white window. WebKitGTK's own disk cache (~/.local/share/
/// com.eqlp.oracle/{WebKitCache,CacheStorage}) is keyed by app
/// identifier, not by app version -- it survives the binary swap
/// underneath it untouched. The just-installed build's index.html
/// references freshly-hashed JS/CSS filenames (every build's own
/// content hash changes); a cached response for the OLD build's
/// index.html (or its asset requests) can still be served after
/// restart, pointing at filenames that don't exist in the new bundle
/// at all -- the page loads, the script never does, nothing ever
/// mounts. This app's own dev loop has had to clear this by hand
/// before every local relaunch all along (same two directories); a
/// real end-user update needs the same treatment automatically, not
/// left to whoever hits the bug to figure out from scratch. Linux/
/// WebKitGTK-specific -- Windows' WebView2 doesn't share this failure
/// mode the same way, and app_data_dir() layout differs there anyway.
/// Best-effort: a failed clear (dir missing, permissions) must never
/// block the restart the update itself already succeeded at.
#[cfg(target_os = "linux")]
fn clear_webview_cache(app: &AppHandle) {
    let Ok(data_dir) = app.path().app_data_dir() else {
        return;
    };
    for sub in ["WebKitCache", "CacheStorage"] {
        let _ = std::fs::remove_dir_all(data_dir.join(sub));
    }
}

#[cfg(not(target_os = "linux"))]
fn clear_webview_cache(_app: &AppHandle) {}

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
    clear_webview_cache(&app);
    app.restart();
}
