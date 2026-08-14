//! The live loop: which file is the game writing to right now, and what
//! does it say. One background thread per configured directory.
//!
//! Design notes: `docs/design/sources.md` -- polling (not filesystem
//! notification), "newest file in the folder" is the only reliable signal
//! for which character is currently logged in, and a `Framer` is required
//! while tailing because a poll can land mid-line.

use crate::ingest::{Ingest, LineCounts, RecentLine};
use eqlp_core::frame::Framer;
use eqlp_source::{identity_from_filename, newest_log_in, Clock, SystemClock, Tail, TailEvent, POLL_MS};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

/// How often to re-scan the directory for a newer `eqlog_*.txt`. Character
/// switches are rare compared to line growth, so this runs far less often
/// than the 250 ms tail poll.
const RESCAN_MS: i64 = 5_000;

/// Emit a tick at least this often even when nothing new arrived, so the UI
/// can show "still watching" instead of going stale-looking mid-lull.
const HEARTBEAT_MS: i64 = 3_000;

/// What the toolbar/Overview module show: which file, whose character,
/// whether we're still replaying history. Cheap to clone; read on every
/// `get_status` and pushed on every tick.
#[derive(Debug, Clone, Default, Serialize)]
pub struct TailStatus {
    pub log_dir: Option<String>,
    pub file: Option<String>,
    pub character: Option<String>,
    pub server: Option<String>,
    pub watching: bool,
    pub tail_status: &'static str,
    /// True while replaying the file's existing content before catching up
    /// to live. The Combat module's numbers are already usable during this
    /// window -- it's the live feed and real-time idle-closing that wait
    /// for it to clear. See `Ingest::mark_live`.
    pub backfilling: bool,
}

#[derive(Clone, Serialize)]
struct ParseTick {
    status: TailStatus,
    counts: LineCounts,
    recent: Vec<RecentLine>,
}

pub struct WorkerHandle {
    stop: Arc<AtomicBool>,
}

impl WorkerHandle {
    /// Signals the worker to exit; does not block. The thread notices within
    /// one poll interval and drops its file handle on the way out.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

pub fn spawn(app: AppHandle, log_dir: PathBuf, ingest: Arc<Mutex<Ingest>>, status: Arc<Mutex<TailStatus>>) -> WorkerHandle {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_flag = stop.clone();
    thread::spawn(move || run(app, log_dir, stop_flag, ingest, status));
    WorkerHandle { stop }
}

fn run(app: AppHandle, log_dir: PathBuf, stop: Arc<AtomicBool>, ingest: Arc<Mutex<Ingest>>, status: Arc<Mutex<TailStatus>>) {
    let engine = match crate::parser::build_engine() {
        Ok(e) => e,
        Err(e) => {
            let _ = app.emit("parse-error", format!("rule pack failed to load: {e}"));
            return;
        }
    };
    let mut matcher = engine.matcher();
    let clock = SystemClock;

    // A fresh directory (first launch, or "change folder") starts from a
    // fresh parsed db -- row indices and encounter ids from a previous
    // directory mean nothing here.
    *ingest.lock().unwrap() = Ingest::default();

    let mut target: Option<PathBuf> = None;
    let mut tail: Option<Tail> = None;
    let mut framer = Framer::default();
    let mut backfilling = false;

    let mut last_rescan = clock.now_ms() - RESCAN_MS;
    let mut last_emit = clock.now_ms() - HEARTBEAT_MS;

    while !stop.load(Ordering::Relaxed) {
        let now = clock.now_ms();
        let mut switched = false;

        if now - last_rescan >= RESCAN_MS {
            last_rescan = now;
            if let Some(newest) = newest_log_in(&log_dir) {
                if target.as_ref() != Some(&newest) {
                    target = Some(newest.clone());
                    // Replay this file's existing content, then continue
                    // live -- "watch live" and "browse past fights" are the
                    // same tail, just before and after catching up.
                    tail = Some(Tail::from_start(newest));
                    framer = Framer::default();
                    backfilling = true;
                    *ingest.lock().unwrap() = Ingest::default();
                    switched = true;
                }
            }
        }

        let mut tail_status: &'static str = "idle";
        if let Some(t) = tail.as_mut() {
            let mut ing = ingest.lock().unwrap();
            let ev = t.poll(|chunk| {
                framer.push(chunk, |line| {
                    let outcome = matcher.classify(line);
                    ing.route(&engine, line, &outcome);
                });
            });
            tail_status = match ev {
                TailEvent::Grew(_) => "grew",
                TailEvent::Truncated => "truncated",
                TailEvent::Replaced => "replaced",
                TailEvent::Missing => "missing",
                TailEvent::Idle => "idle",
            };
            // The file's own content changed identity mid-tail (not a
            // character switch, which already gets a fresh Ingest above).
            // A carried partial line from before means nothing now; the
            // parsed history in the store is not thrown away for it --
            // only the frame boundary resets, at the cost of possibly one
            // misframed line at the seam. Rare enough, and cheap enough to
            // accept, that rebuilding hours of parsed history over it
            // would be the wrong trade.
            if matches!(ev, TailEvent::Truncated | TailEvent::Replaced) {
                framer = Framer::default();
            }
            if backfilling {
                ing.mark_live();
                backfilling = false;
            }
            ing.tick(now);
        }

        let has_news = switched || !matches!(tail_status, "idle" | "missing");
        if has_news || now - last_emit >= HEARTBEAT_MS {
            last_emit = now;
            let identity = target.as_ref().and_then(|p| identity_from_filename(p));
            let st = TailStatus {
                log_dir: Some(log_dir.display().to_string()),
                file: target.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()),
                character: identity.as_ref().map(|(c, _)| c.clone()),
                server: identity.as_ref().map(|(_, s)| s.clone()),
                watching: true,
                tail_status,
                backfilling,
            };
            *status.lock().unwrap() = st.clone();

            let mut ing = ingest.lock().unwrap();
            let counts = ing.counts.clone();
            let recent = std::mem::take(&mut ing.recent);
            drop(ing);
            let _ = app.emit("parse-tick", ParseTick { status: st, counts, recent });
        }

        thread::sleep(Duration::from_millis(POLL_MS));
    }
}
