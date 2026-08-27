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
    let mut builder = app
        .updater_builder()
        .endpoints(vec![url])
        .map_err(|e| e.to_string())?;
    // why: real bug, caught live -- a SECOND install froze white after a
    // FIRST install's own WebKitGTK-cache-clear fix already shipped, so
    // clearing that cache alone wasn't the whole story. tauri-plugin-
    // updater's own extract_path resolution on Linux is bare
    // current_exe() (confirmed straight off its source, no AppImage
    // special-casing at all) -- for a real running AppImage that's
    // /proc/self/exe, which resolves through the FUSE mount squashfs
    // extracts itself into (confirmed live: readlink on a real running
    // instance showed /tmp/.mount_eqlp-XXXXXX/usr/bin/eqlp-app), NOT the
    // actual persistent .AppImage file the user downloaded and launches
    // -- an ephemeral, almost certainly read-only path that gets torn
    // down the moment this process exits. install_appimage's own
    // std::fs::write targets THAT path, not the real file. $APPIMAGE is
    // the real fix -- every genuine type2 AppImage runtime sets it to
    // the actual outer file's own absolute path before ever execing
    // into the mount, so it's exactly the extract_path this crate
    // should have used already. Only overridden when actually present
    // (running as a real AppImage) -- a .deb/.rpm install's own
    // current_exe() is already correct, nothing to fix there.
    #[cfg(target_os = "linux")]
    if let Ok(appimage) = std::env::var("APPIMAGE") {
        builder = builder.executable_path(appimage);
    }
    let updater = builder.build().map_err(|e| e.to_string())?;
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

/// why: Spencer's own real bug reports, twice -- see install_pending_update's
/// own doc for why clearing WebKitGTK's cache DURING the old process's own
/// shutdown was itself the actual race (its WebKit subprocesses -- WebProcess/
/// NetworkProcess/GPU, separate OS processes, not just threads -- still fully
/// alive and holding open handles into exactly the directories being
/// deleted). Run once per real app start instead, from the fresh process's
/// own setup(), before any window/webview of its own has been created yet --
/// nothing else alive to race with. Gated on a real version comparison (a
/// plain marker file, not a preference -- this isn't user-facing state, just
/// bookkeeping) so an ordinary launch with nothing new installed never
/// touches a perfectly good cache; only a version that's actually different
/// from the last real launch -- an in-app update, or Spencer swapping in a
/// new build by hand -- clears it. Same underlying staleness this was always
/// about: WebKitGTK's own cache is keyed by app identifier, not app version,
/// so a hashed-asset-filename mismatch between what's cached and what THIS
/// build actually ships is exactly what a version change here means.
#[cfg(target_os = "linux")]
pub fn clear_stale_webview_cache_if_needed(app: &AppHandle) {
    let Ok(data_dir) = app.path().app_data_dir() else {
        return;
    };
    let marker = data_dir.join(".last_run_version");
    let current = app.package_info().version.to_string();
    if std::fs::read_to_string(&marker).ok().as_deref() == Some(current.as_str()) {
        return;
    }
    for sub in ["WebKitCache", "CacheStorage"] {
        let _ = std::fs::remove_dir_all(data_dir.join(sub));
    }
    let _ = std::fs::create_dir_all(&data_dir);
    let _ = std::fs::write(&marker, &current);
}

#[cfg(not(target_os = "linux"))]
pub fn clear_stale_webview_cache_if_needed(_app: &AppHandle) {}

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
    // why: real bug, caught TWICE live -- clearing WebKitGTK's own cache
    // used to happen right here, before restart, in the OLD process --
    // while its own WebKit subprocesses (WebProcess/NetworkProcess/GPU,
    // separate OS processes, not just threads) were still fully alive
    // and actively holding open handles into exactly the directories
    // being deleted. Spencer's own follow-up report -- a SECOND white
    // page, this time on the plain local binary, not even the AppImage
    // path the first fix targeted, and "not frozen, the update pipeline
    // just broke" (a live-but-blank webview, not a hang) -- doesn't fit
    // a stale-cache-alone explanation a second time; it fits a race
    // between the dying old process's own WebKit subprocesses and the
    // freshly-restarted one's own fresh subprocesses fighting over the
    // same cache directory an instant after `rm -rf` pulled it out from
    // under the still-running old ones. See app_startup's own
    // clear_stale_webview_cache -- moved to run once, at the START of
    // the NEXT process's own setup(), before any webview/window of its
    // own has been created at all, nothing else alive to race with.
    app.restart();
}
