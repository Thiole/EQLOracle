//! The live loop: which file is the game writing to right now, and what
//! does it say. One background thread per configured directory.
//!
//! Design notes: `docs/design/sources.md` -- polling (not filesystem
//! notification), "newest file in the folder" is the only reliable signal
//! for which character is currently logged in, and a `Framer` is required
//! while tailing because a poll can land mid-line.

use crate::ingest::{self, Ingest, LineCounts, RecentLine};
use crate::inventory::{inventory_character, is_inventory_dump};
use eqlp_core::frame::Framer;
use eqlp_source::{
    identity_from_filename, newest_log_in, Clock, SystemClock, Tail, TailEvent, POLL_MS,
};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

/// why: directory rescan interval; character switches are rare vs line growth
const RESCAN_MS: i64 = 5_000;

/// why: minimum tick cadence so the UI never looks stale mid-lull
const HEARTBEAT_MS: i64 = 3_000;

/// why: bounds one lock hold to one chunk -- replaying a multi-million-line
/// file in one call would block `get_status` for the whole replay with no
/// progress tick until done
const BACKFILL_CHUNK_LINES: usize = 100_000;

/// why: toolbar/Overview status -- cheap to clone, read every `get_status`
#[derive(Debug, Clone, Default, Serialize)]
pub struct TailStatus {
    pub log_dir: Option<String>,
    pub file: Option<String>,
    pub character: Option<String>,
    pub server: Option<String>,
    pub watching: bool,
    pub tail_status: &'static str,
    /// why: true while replaying existing content; Combat numbers already
    /// usable during this window, only live feed/idle-closing wait for it
    pub backfilling: bool,
    /// why: pets auto-attributed so far, shown so inference stays visible
    pub pets_attributed: usize,
}

#[derive(Clone, Serialize)]
struct ParseTick {
    status: TailStatus,
    counts: LineCounts,
    recent: Vec<RecentLine>,
}

/// why: what the inv-toast listens for; `character` from filename, best-effort
#[derive(Clone, Serialize)]
struct InventoryDumpEvent {
    file: String,
    character: Option<String>,
}

fn tail_event_str(ev: TailEvent) -> &'static str {
    match ev {
        TailEvent::Grew(_) => "grew",
        TailEvent::Truncated => "truncated",
        TailEvent::Replaced => "replaced",
        TailEvent::Missing => "missing",
        TailEvent::Idle => "idle",
    }
}

pub struct WorkerHandle {
    stop: Arc<AtomicBool>,
}

impl WorkerHandle {
    /// why: signals exit without blocking; thread notices within one poll interval
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

pub fn spawn(
    app: AppHandle,
    log_dir: PathBuf,
    ingest: Arc<Mutex<Ingest>>,
    status: Arc<Mutex<TailStatus>>,
) -> WorkerHandle {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_flag = stop.clone();
    thread::spawn(move || run(app, log_dir, stop_flag, ingest, status));
    WorkerHandle { stop }
}

fn run(
    app: AppHandle,
    log_dir: PathBuf,
    stop: Arc<AtomicBool>,
    ingest: Arc<Mutex<Ingest>>,
    status: Arc<Mutex<TailStatus>>,
) {
    let engine = match crate::parser::build_engine() {
        Ok(e) => e,
        Err(e) => {
            let _ = app.emit("parse-error", format!("rule pack failed to load: {e}"));
            return;
        }
    };
    let mut matcher = engine.matcher();
    let clock = SystemClock;
    // why: backfill parses in parallel, live growth stays single-threaded.
    // 2x detected cores, not just the raw count -- re-measured live via
    // examples/backfill_bench.rs's own thread-count sweep against a real
    // 265MB/3.3M-line log: the curve keeps improving well past
    // available_parallelism() (12 real threads clearly beats 6, 32 beat
    // 24 by a hair too -- this is mixed regex-CPU work with brief
    // per-chunk lock contention, not purely compute-bound, so
    // oversubscribing a bit keeps cores fed during those stalls). 32 is
    // the real ceiling that sweep found -- returns essentially flat
    // past it (24->32 saved only 0.05s of a 4.28s run).
    let backfill_threads = thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .saturating_mul(2)
        .min(32);

    // why: fresh directory starts fresh -- old row/encounter ids mean nothing here
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
                    // why: "watch live" and "browse past fights" are the same
                    // tail, just before/after catching up
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
            if backfilling {
                // why: `Tail::poll` loops to current EOF, so this one call
                // is "everything on disk" -- parsed with a bounded thread
                // pool, not the incremental single-line live path
                let mut raw = Vec::new();
                let ev = t.poll(|chunk| raw.extend_from_slice(chunk));
                tail_status = tail_event_str(ev);

                // why: framed once, fed in bounded pieces -- see BACKFILL_CHUNK_LINES
                let lines = ingest::framed_lines(&raw);
                for chunk in lines.chunks(BACKFILL_CHUNK_LINES.max(1)) {
                    if stop.load(Ordering::Relaxed) {
                        return; // directory changed, or the app is closing, mid-replay
                    }
                    {
                        let mut ing = ingest.lock().unwrap();
                        ingest::backfill_lines(&mut ing, &engine, chunk, backfill_threads);
                    }
                    // why: unconditional -- this is the counting-up progress the UI shows
                    last_emit = clock.now_ms();
                    emit_tick(&app, &log_dir, &target, tail_status, true, &ingest, &status);
                }

                let mut ing = ingest.lock().unwrap();
                ing.mark_live();
                ing.tick(now);
                drop(ing);
                backfilling = false;
            } else {
                let mut ing = ingest.lock().unwrap();
                let ev = t.poll(|chunk| {
                    framer.push(chunk, |line| {
                        let outcome = matcher.classify(line);
                        ing.route(&engine, line, &outcome);
                    });
                });
                tail_status = tail_event_str(ev);
                // why: file changed identity mid-tail -- only the frame
                // boundary resets (possibly one misframed seam line), not
                // the parsed history; rebuilding it would be the wrong trade
                if matches!(ev, TailEvent::Truncated | TailEvent::Replaced) {
                    framer = Framer::default();
                }
                ing.tick(now);
            }
        }

        let has_news = switched || !matches!(tail_status, "idle" | "missing");
        if has_news || now - last_emit >= HEARTBEAT_MS {
            last_emit = now;
            emit_tick(
                &app,
                &log_dir,
                &target,
                tail_status,
                backfilling,
                &ingest,
                &status,
            );
        }

        thread::sleep(Duration::from_millis(POLL_MS));
    }
}

/// why: builds/emits one `parse-tick`; shared by backfill chunk loop and
/// steady-state heartbeat so both paths build it identically
fn emit_tick(
    app: &AppHandle,
    log_dir: &Path,
    target: &Option<PathBuf>,
    tail_status: &'static str,
    backfilling: bool,
    ingest: &Arc<Mutex<Ingest>>,
    status: &Arc<Mutex<TailStatus>>,
) {
    let identity = target.as_ref().and_then(|p| identity_from_filename(p));
    let pets_attributed = ingest.lock().unwrap().pet_owner_count();
    let st = TailStatus {
        log_dir: Some(log_dir.display().to_string()),
        file: target
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned()),
        character: identity.as_ref().map(|(c, _)| c.clone()),
        server: identity.as_ref().map(|(_, s)| s.clone()),
        watching: true,
        tail_status,
        backfilling,
        pets_attributed,
    };
    *status.lock().unwrap() = st.clone();

    let mut ing = ingest.lock().unwrap();
    let counts = ing.counts.clone();
    let recent = std::mem::take(&mut ing.recent);
    let finished = std::mem::take(&mut ing.pending_history);
    let inventory_files = std::mem::take(&mut ing.pending_inventory_files);
    let notifications = std::mem::take(&mut ing.pending_notifications);
    // why: one lock hold; None unless save_profile is on, avoids paying
    // for reconciliation work on every tick when the feature is off
    let profile_classes = crate::preferences::load(app)
        .save_profile
        .then(|| crate::combat::class_configurations(&ing, "You"))
        .and_then(|dto| dto.configurations.into_iter().next())
        .map(|c| c.classes);
    drop(ing);

    if !notifications.is_empty() {
        // why: loaded once per tick; backend owns whether a disabled kind
        // reaches the frontend at all, not the log line itself
        let settings = crate::settings::load(app);
        for n in notifications
            .into_iter()
            .filter(|n| settings.is_enabled(&n.kind))
        {
            let _ = app.emit("notification", n);
        }
    }
    for record in &finished {
        // why: best-effort -- a write failure must not interrupt live tailing
        if let Err(e) = crate::history::append(app, record) {
            eprintln!("parse history write failed for {}: {e}", record.target);
        }
    }
    // why: same best-effort stance -- a failed profile save must not
    // interrupt live tailing; character is None before first zone/cast line
    if let (Some(classes), Some(character)) = (&profile_classes, &st.character) {
        if let Err(e) = crate::profile::save_if_changed(app, character, classes) {
            eprintln!("profile save failed for {character}: {e}");
        }
    }
    // why: only readable dumps notify; frontend fetches via IPC
    for file in inventory_files.iter().filter(|f| is_inventory_dump(f)) {
        let _ = app.emit(
            "inventory-dump",
            InventoryDumpEvent {
                file: file.clone(),
                character: inventory_character(file),
            },
        );
    }
    let _ = app.emit(
        "parse-tick",
        ParseTick {
            status: st,
            counts,
            recent,
        },
    );
}
