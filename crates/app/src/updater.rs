//! why: in-app update check/install -- no redirect to GitHub to grab the
//! newest release by hand. Endpoint is built fresh per check from the
//! user's own saved channel preference (see `preferences::UpdateChannel`),
//! not the static default in tauri.conf.json (that's only the Public
//! fallback for the rare caller that skips this and uses the plugin directly).

use crate::preferences::{self, UpdateChannel};
use crate::state::{AppState, LockRecover};
use serde::Serialize;
#[cfg(target_os = "linux")]
use tauri::Manager;
use tauri::{AppHandle, Emitter, State};
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

/// why: the update prompt should link to the real GitHub release/
/// changelog, not just show notes text with nowhere to read more
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
    /// why: a real link to the GitHub release page, not just notes text
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
    *state.pending_update.lock_recover() = found;
    Ok(dto)
}

/// why: see install_pending_update's doc for why clearing WebKitGTK's
/// cache DURING the old process's shutdown was itself a race (its
/// WebProcess/NetworkProcess/GPU subprocesses still alive, holding open
/// handles into the directories being deleted). Run once per app start
/// instead, from the fresh process's setup(), before any window/webview
/// exists -- nothing else alive to race with. Gated on a version
/// comparison (plain marker file, not a preference -- bookkeeping, not
/// user-facing) so an ordinary launch never touches a good cache; only
/// a version different from the last launch clears it. WebKitGTK's
/// cache is keyed by app identifier, not app version, so a hashed-
/// asset-filename mismatch is exactly what a version change here means.
#[cfg(target_os = "linux")]
pub fn clear_stale_webview_cache_if_needed(app: &AppHandle) {
    let Ok(data_dir) = app.path().app_data_dir() else {
        return;
    };
    let marker = data_dir.join(".last_run_version");
    let current = app.package_info().version.to_string();
    // why: a version change isn't the only way the cache goes bad -- a
    // killed instance (real incident: a host memory policy SIGKILLing
    // the app mid-run) leaves WebKitGTK's cache torn mid-write, and the
    // next same-version launch rendered a white window with no error.
    // The sentinel is written every startup and removed only by a clean
    // exit (mark_clean_exit) -- present here means the last run died
    // hard, so the cache can't be trusted regardless of version.
    let unclean = data_dir.join(".unclean_exit");
    let version_changed =
        std::fs::read_to_string(&marker).ok().as_deref() != Some(current.as_str());
    if version_changed || unclean.exists() {
        for sub in ["WebKitCache", "CacheStorage"] {
            let _ = std::fs::remove_dir_all(data_dir.join(sub));
        }
    }
    let _ = std::fs::create_dir_all(&data_dir);
    let _ = std::fs::write(&marker, &current);
    let _ = std::fs::write(&unclean, b"");
}

#[cfg(not(target_os = "linux"))]
pub fn clear_stale_webview_cache_if_needed(_app: &AppHandle) {}

/// why: the clean-exit half of the sentinel above -- called from the
/// main window's own close path, the one deliberate way this app exits
#[cfg(target_os = "linux")]
pub fn mark_clean_exit(app: &AppHandle) {
    if let Ok(data_dir) = app.path().app_data_dir() {
        let _ = std::fs::remove_file(data_dir.join(".unclean_exit"));
    }
}

#[cfg(not(target_os = "linux"))]
pub fn mark_clean_exit(_app: &AppHandle) {}

/// why: consumes whatever check_for_update last found -- an explicit
/// error (not a silent no-op) if the frontend calls this without a real
/// check first, or the check found nothing to install
pub async fn install_pending_update(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let update = state.pending_update.lock_recover().clone();
    let Some(update) = update else {
        return Err("no pending update -- call check_for_update first".to_string());
    };
    // why: real incident, destroyed a real install -- a CI manifest bug
    // put the RPM's url under linux-x86_64, and download_and_install
    // wrote that RPM byte-for-byte over the running $APPIMAGE file: the
    // app closed for its restart and could never reopen. The manifest
    // side is fixed in CI (3-release.yml's finalize job), but the
    // client must never trust that alone: when this instance runs as an
    // AppImage, only an AppImage payload may ever be installed.
    #[cfg(target_os = "linux")]
    if std::env::var("APPIMAGE").is_ok() {
        let url = update.download_url.as_str();
        if !url.ends_with(".AppImage") && !url.ends_with(".AppImage.tar.gz") {
            return Err(format!(
                "refusing to install: this instance runs as an AppImage but the update \
                 manifest points at a non-AppImage payload ({url}) -- installing it would \
                 overwrite the AppImage file itself. The release manifest is broken; \
                 report this and update manually from the Releases page."
            ));
        }
    }
    // why: real progress, not a spinner -- the download runs 10-15s with
    // the old window still up, and with empty callbacks that period read
    // as "nothing happening" (player's own report). (received, total)
    // per chunk; total is None when the server sends no content-length.
    let progress_app = app.clone();
    let mut received: u64 = 0;
    update
        .download_and_install(
            move |chunk, total| {
                received += chunk as u64;
                let _ = progress_app.emit("update-progress", (received, total));
            },
            || {},
        )
        .await
        .map_err(|e| e.to_string())?;
    // why: deliberately NO restart here -- two-step flow, player's own
    // spec: install in the background, then "update installed, restart
    // when ready" (restart_app is that second step; an ordinary window
    // close works too, the next launch is the new version either way).
    // Deferring is safe on Linux because the plugin swaps the AppImage
    // via rename -- the running instance's FUSE mount keeps the old
    // inode. On Windows this line is never reached at all: the plugin
    // launches the installer and exits the process itself
    // (std::process::exit(0) in its own install path), so install
    // remains install-and-exit there inherently.
    //
    // The WebKitGTK cache-clear that used to be discussed here still
    // runs at the start of the NEXT process's setup() (see
    // clear_stale_webview_cache_if_needed) -- unaffected by who
    // triggers the restart or when.
    Ok(())
}

/// why: the deferred second step of install_pending_update -- the
/// "restart now" button once an update is installed. Never returns.
pub fn restart_app(app: AppHandle) {
    app.restart();
}
