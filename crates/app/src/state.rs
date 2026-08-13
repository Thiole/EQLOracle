//! Shared app state, managed by Tauri and reached from every command.

use crate::config::AppConfig;
use crate::tail_worker::{Snapshot, WorkerHandle};
use std::sync::{Arc, Mutex};

pub struct AppState {
    pub config: Mutex<Option<AppConfig>>,
    /// The running tail thread, if a directory has been picked. Replaced
    /// (old one told to stop) whenever the user changes folders.
    pub worker: Mutex<Option<WorkerHandle>>,
    /// Latest snapshot, written by the worker thread and read by
    /// `get_status` for first paint -- so a reload never shows zeros while
    /// waiting on the next tick.
    pub snapshot: Arc<Mutex<Snapshot>>,
}

impl AppState {
    pub fn new() -> Self {
        AppState { config: Mutex::new(None), worker: Mutex::new(None), snapshot: Arc::new(Mutex::new(Snapshot::default())) }
    }
}

impl Default for AppState {
    fn default() -> Self {
        AppState::new()
    }
}
