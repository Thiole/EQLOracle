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
}

impl AppState {
    pub fn new() -> Self {
        AppState {
            config: Mutex::new(None),
            worker: Mutex::new(None),
            ingest: Arc::new(Mutex::new(Ingest::default())),
            status: Arc::new(Mutex::new(TailStatus::default())),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        AppState::new()
    }
}
