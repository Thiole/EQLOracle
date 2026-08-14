//! Shared app state, managed by Tauri and reached from every command.

use crate::config::AppConfig;
use crate::ingest::Ingest;
use crate::tail_worker::{TailStatus, WorkerHandle};
use std::sync::{Arc, Mutex};

pub struct AppState {
    pub config: Mutex<Option<AppConfig>>,
    /// The running tail thread, if a directory has been picked. Replaced
    /// (old one told to stop) whenever the user changes folders.
    pub worker: Mutex<Option<WorkerHandle>>,
    /// The parsed db: every event classified from the current tail file,
    /// plus the encounter graph and zone spans built from it. Written by
    /// the worker thread, queried directly by the Combat module's commands
    /// -- no round trip through the worker needed to answer a query.
    pub ingest: Arc<Mutex<Ingest>>,
    /// Lightweight status (file/character/server/watching), separate from
    /// `ingest` so a toolbar repaint never waits on a combat query and vice
    /// versa.
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
