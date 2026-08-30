//! Shared app state, managed by Tauri and reached from every command.

use crate::config::AppConfig;
use crate::ingest::Ingest;
use crate::tail_worker::{TailStatus, WorkerHandle};
use std::sync::{Arc, Mutex, MutexGuard};

/// why: a poisoned mutex must NOT brick the app. Every command locks
/// `ingest`, and the tail worker calls `backfill_lines` while holding
/// it -- one panicking parse line would poison the mutex, after which
/// every `lock().unwrap()` in every command panics forever, leaving the
/// window open but permanently dead ("app not usable after a little
/// bit"). A panic leaves the data memory-safe (Rust's guarantee), only
/// logically mid-update, so recovering the guard and carrying on is the
/// right call -- the next full-log replay on restart rebuilds cleanly
/// anyway. Pairs with the worker's own catch_unwind, which stops the
/// poison at its source; this is the belt to that suspenders.
pub trait LockRecover<T: ?Sized> {
    fn lock_recover(&self) -> MutexGuard<'_, T>;
}

impl<T: ?Sized> LockRecover<T> for Mutex<T> {
    fn lock_recover(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

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
}

impl AppState {
    pub fn new() -> Self {
        AppState {
            config: Mutex::new(None),
            worker: Mutex::new(None),
            ingest: Arc::new(Mutex::new(Ingest::default())),
            status: Arc::new(Mutex::new(TailStatus::default())),
            pending_update: Mutex::new(None),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        AppState::new()
    }
}

#[cfg(test)]
mod poison_tests {
    use super::*;
    use std::sync::Arc;

    /// why: the core guarantee -- a thread panicking while holding the
    /// lock (exactly what a bad parse line inside the worker would do)
    /// poisons the mutex, and every later command must still get the
    /// data back instead of cascading panics
    #[test]
    fn a_poisoned_mutex_still_hands_back_its_guard() {
        let m = Arc::new(Mutex::new(vec![1, 2, 3]));
        let m2 = Arc::clone(&m);
        let _ = std::thread::spawn(move || {
            let mut g = m2.lock().unwrap();
            g.push(4);
            panic!("die while holding the lock");
        })
        .join();
        // std lock() would now return Err(Poisoned) forever
        assert!(m.lock().is_err(), "precondition: mutex is poisoned");
        // lock_recover recovers the guard AND the data written before the panic
        let g = m.lock_recover();
        assert_eq!(*g, vec![1, 2, 3, 4]);
    }
}
