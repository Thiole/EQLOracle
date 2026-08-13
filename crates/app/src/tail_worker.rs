//! The live loop: which file is the game writing to right now, and what
//! does it say. One background thread per configured directory.
//!
//! Design notes: `docs/design/sources.md` -- polling (not filesystem
//! notification), "newest file in the folder" is the only reliable signal
//! for which character is currently logged in, and a `Framer` is required
//! while tailing because a poll can land mid-line.

use crate::parser::{build_engine, Counts, RecentLine};
use eqlp_core::frame::Framer;
use eqlp_core::Outcome;
use eqlp_source::{identity_from_filename, newest_log_in, Clock, SystemClock, Tail, TailEvent, POLL_MS};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

/// How often to re-scan the directory for a newer `eqlog_*.txt`. Character
/// switches are rare compared to line growth, so this runs far less often
/// than the 250 ms tail poll -- it's a directory listing, not a big cost,
/// but there is no reason to pay it four times a second.
const RESCAN_MS: i64 = 5_000;

/// Emit a tick at least this often even when nothing new arrived, so the UI
/// can show "still watching" instead of going stale-looking mid-lull.
const HEARTBEAT_MS: i64 = 3_000;

/// Feed rows kept server-side between emits. The frontend keeps its own
/// (smaller) rolling window on top of this; this cap just bounds one
/// worker's memory if the UI is slow to drain a burst.
const MAX_PENDING_RECENT: usize = 500;

/// Snapshot of "what's true right now", handed to the frontend both by push
/// (the `parse-tick` event) and by pull (`get_status`, for first paint /
/// reconnect without waiting on the next tick).
#[derive(Debug, Clone, Default, Serialize)]
pub struct Snapshot {
    pub log_dir: Option<String>,
    pub file: Option<String>,
    pub character: Option<String>,
    pub server: Option<String>,
    pub watching: bool,
    pub tail_status: &'static str,
    pub total: u64,
    pub matched: u64,
    pub unmatched: u64,
    pub headerless: u64,
    pub blank: u64,
    pub by_kind: BTreeMap<String, u64>,
}

#[derive(Clone, Serialize)]
struct ParseTick {
    snapshot: Snapshot,
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

pub fn spawn(app: AppHandle, log_dir: PathBuf, shared: Arc<Mutex<Snapshot>>) -> WorkerHandle {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_flag = stop.clone();
    thread::spawn(move || run(app, log_dir, stop_flag, shared));
    WorkerHandle { stop }
}

fn run(app: AppHandle, log_dir: PathBuf, stop: Arc<AtomicBool>, shared: Arc<Mutex<Snapshot>>) {
    let engine = match build_engine() {
        Ok(e) => e,
        Err(e) => {
            let _ = app.emit("parse-error", format!("rule pack failed to load: {e}"));
            return;
        }
    };
    let mut matcher = engine.matcher();
    let clock = SystemClock;

    let mut target: Option<PathBuf> = None;
    let mut tail: Option<Tail> = None;
    let mut framer = Framer::default();
    let mut counts = Counts::default();
    let mut recent: Vec<RecentLine> = Vec::new();

    // Force an immediate scan and an immediate first emit on entry.
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
                    // Live only: pick up whatever the game writes from here.
                    // History backfill is `Tail::from_start`, a separate,
                    // deliberately opt-in feature -- not what "which file is
                    // the game logging to right now" asks for.
                    tail = Some(Tail::at_end(newest));
                    framer = Framer::default();
                    counts = Counts::default();
                    recent.clear();
                    switched = true;
                }
            }
        }

        let mut tail_status: &'static str = "idle";
        if let Some(t) = tail.as_mut() {
            let mut raw = Vec::new();
            let ev = t.poll(|chunk| raw.extend_from_slice(chunk));
            tail_status = match ev {
                TailEvent::Grew(_) => "grew",
                TailEvent::Truncated => "truncated",
                TailEvent::Replaced => "replaced",
                TailEvent::Missing => "missing",
                TailEvent::Idle => "idle",
            };

            // The file's own content changed identity; a carried partial
            // line from before no longer means anything, and counts so far
            // described bytes that are no longer there.
            if matches!(ev, TailEvent::Truncated | TailEvent::Replaced) {
                framer = Framer::default();
                counts = Counts::default();
                recent.clear();
            }

            if !raw.is_empty() {
                framer.push(&raw, |line| {
                    let outcome = matcher.classify(line);
                    let kind = match &outcome {
                        Outcome::Matched(m) => Some(engine.rule(m.rule).kind.as_str()),
                        _ => None,
                    };
                    counts.record(&outcome, kind);
                    if let Outcome::Matched(m) = &outcome {
                        let rule = engine.rule(m.rule);
                        recent.push(RecentLine {
                            kind: rule.kind.clone(),
                            rule_id: rule.id.clone(),
                            text: String::from_utf8_lossy(m.body.slice(line)).into_owned(),
                        });
                        if recent.len() > MAX_PENDING_RECENT {
                            let excess = recent.len() - MAX_PENDING_RECENT;
                            recent.drain(0..excess);
                        }
                    }
                });
            }
        }

        let has_news = switched || !matches!(tail_status, "idle" | "missing") || !recent.is_empty();
        if has_news || now - last_emit >= HEARTBEAT_MS {
            last_emit = now;
            let identity = target.as_ref().and_then(|p| identity_from_filename(p));
            let snapshot = Snapshot {
                log_dir: Some(log_dir.display().to_string()),
                file: target.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()),
                character: identity.as_ref().map(|(c, _)| c.clone()),
                server: identity.as_ref().map(|(_, s)| s.clone()),
                watching: true,
                tail_status,
                total: counts.total,
                matched: counts.matched,
                unmatched: counts.unmatched,
                headerless: counts.headerless,
                blank: counts.blank,
                by_kind: counts.by_kind.clone(),
            };
            *shared.lock().unwrap() = snapshot.clone();
            let batch = std::mem::take(&mut recent);
            let _ = app.emit("parse-tick", ParseTick { snapshot, recent: batch });
        }

        thread::sleep(Duration::from_millis(POLL_MS));
    }
}
