//! Shared app state, managed by Tauri and reached from every command.

use crate::config::AppConfig;
use crate::ingest::Ingest;
use crate::tail_worker::{TailStatus, WorkerHandle};
use std::sync::{Arc, Mutex};

pub struct AppState {
    pub config: Mutex<Option<AppConfig>>,
    /// why: replaced (old one stopped) whenever the user changes folders
    pub worker: Mutex<Option<WorkerHandle>>,
    /// why: written by worker thread, queried directly by Combat commands
    pub ingest: Arc<Mutex<Ingest>>,
    /// why: separate from `ingest` so a toolbar repaint never blocks on a query
    pub status: Arc<Mutex<TailStatus>>,
    /// why: bridges check_for_update -> install_pending_update, two
    /// separate commands (a confirm prompt sits between them) -- the
    /// Update itself carries the download URL/signature, no reason to
    /// re-check just to install what was already found
    pub pending_update: Mutex<Option<tauri_plugin_updater::Update>>,
    /// why: the real overlay window is one shared window holding every
    /// enabled widget -- must stay open as long as ANY widget wants it,
    /// close only once none do (see commands::set_overlay_enabled's own
    /// doc). The overlay window itself is a separate webview/JS realm
    /// (see ui's OverlayApp.svelte) that can't see the main window's
    /// local Svelte stores, so which widgets are live has to be real
    /// backend state it can query/be told about, not something inferred
    /// client-side.
    pub overlay_widgets: Mutex<std::collections::HashSet<String>>,
}

impl AppState {
    pub fn new() -> Self {
        AppState {
            config: Mutex::new(None),
            worker: Mutex::new(None),
            ingest: Arc::new(Mutex::new(Ingest::default())),
            status: Arc::new(Mutex::new(TailStatus::default())),
            pending_update: Mutex::new(None),
            overlay_widgets: Mutex::new(std::collections::HashSet::new()),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        AppState::new()
    }
}
