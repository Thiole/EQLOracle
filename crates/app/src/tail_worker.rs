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

/// How often to re-scan the directory for a newer `eqlog_*.txt`. Character
/// switches are rare compared to line growth, so this runs far less often
/// than the 250 ms tail poll.
const RESCAN_MS: i64 = 5_000;

/// Emit a tick at least this often even when nothing new arrived, so the UI
/// can show "still watching" instead of going stale-looking mid-lull.
const HEARTBEAT_MS: i64 = 3_000;

/// How many lines `backfill_lines` processes per call during history
/// replay. A multi-million-line file replayed in one call holds
/// `ingest`'s lock (and blocks the very first `get_status`, which is what
/// lets the app show its main screen at all) for the file's *entire*
/// replay, with no progress tick emitted until it's already done -- the
/// app would sit on a blank window, then jump straight to "fully caught
/// up" with no number ever counting up in between. Chunking bounds one
/// lock hold to one chunk's worth of work and lets a tick land between
/// chunks; small enough that the app shows up and starts counting almost
/// immediately, large enough that per-chunk overhead (lock acquire, one
/// `parse-tick` emit) stays negligible next to the work it's wrapping.
const BACKFILL_CHUNK_LINES: usize = 100_000;

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
    /// How many pets have been auto-attributed to an owner so far
    /// (`Ingest::note_actor`) -- shown so that inference stays visible
    /// rather than an invisible thing happening to the numbers.
    pub pets_attributed: usize,
}

#[derive(Clone, Serialize)]
struct ParseTick {
    status: TailStatus,
    counts: LineCounts,
    recent: Vec<RecentLine>,
}

/// What the inv-toast listens for (`ui/app/app.js`) -- one per freshly
/// written `/outputfile inventory` dump, named by an `outputfile.complete`
/// line. `character` is read off the filename itself
/// (`<Character>_<zone>-Inventory.txt`, confirmed against a real dump --
/// see `inventory.rs`'s own doc), not looked up anywhere, so it's a
/// best-effort label rather than a guaranteed-correct one.
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
    /// Signals the worker to exit; does not block. The thread notices within
    /// one poll interval and drops its file handle on the way out.
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
    // A backfill (replaying a file's existing content) parses in parallel;
    // live growth is a few lines at a time and stays on the simple
    // single-threaded path. See ingest::backfill_lines.
    //
    // why: 16-thread cap, re-measured via examples/backfill_bench.rs
    let backfill_threads = thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(16);

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
            if backfilling {
                // First poll of a freshly opened file: `Tail::poll` already
                // loops internally until it hits current EOF, so this one
                // call *is* "everything on disk right now" -- pull it into
                // one buffer and parse it with a bounded thread pool
                // instead of the incremental single-line path live growth
                // uses. See ingest::backfill_lines for why that split
                // is worth it.
                let mut raw = Vec::new();
                let ev = t.poll(|chunk| raw.extend_from_slice(chunk));
                tail_status = tail_event_str(ev);

                // Framed once, then fed through in bounded pieces -- see
                // BACKFILL_CHUNK_LINES's doc for why one call over the
                // whole file would leave the UI staring at a blank screen
                // with no progress tick until backfill was already done.
                let lines = ingest::framed_lines(&raw);
                for chunk in lines.chunks(BACKFILL_CHUNK_LINES.max(1)) {
                    if stop.load(Ordering::Relaxed) {
                        return; // directory changed, or the app is closing, mid-replay
                    }
                    {
                        let mut ing = ingest.lock().unwrap();
                        ingest::backfill_lines(&mut ing, &engine, chunk, backfill_threads);
                    }
                    // Unconditional, not gated by the heartbeat throttle
                    // below -- this *is* the counting-up progress the app
                    // shows while replaying history.
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
                // The file's own content changed identity mid-tail (not a
                // character switch, which already gets a fresh Ingest
                // above). A carried partial line from before means nothing
                // now; the parsed history in the store is not thrown away
                // for it -- only the frame boundary resets, at the cost of
                // possibly one misframed line at the seam. Rare enough, and
                // cheap enough to accept, that rebuilding hours of parsed
                // history over it would be the wrong trade.
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

/// Builds and emits one `parse-tick`: refreshed `TailStatus`, the store's
/// running counts, whatever live-feed lines and finished-encounter records
/// piled up since the last tick, and a best-effort append of each finished
/// record to `parse_history.jsonl`. Shared by the backfill chunk loop
/// (fires after *every* chunk, unconditionally -- that's the counting-up
/// progress the app shows while replaying history) and the steady-state
/// heartbeat (fires on real news or every `HEARTBEAT_MS`), so the two paths
/// can never build a `TailStatus`/`ParseTick` differently.
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
    // why: computed under the same lock hold as everything else above
    // (one `ingest.lock()`, not a second just for this) -- `None` unless
    // `save_profile` is on, so a feature that's off by default never pays
    // for `class_configurations`'s own reconciliation work on every tick.
    // See `preferences::Preferences::save_profile`'s own doc for the full
    // policy this is one half of.
    let profile_classes = crate::preferences::load(app)
        .save_profile
        .then(|| crate::combat::class_configurations(&ing, "You"))
        .and_then(|dto| dto.configurations.into_iter().next())
        .map(|c| c.classes);
    drop(ing);

    if !notifications.is_empty() {
        // Loaded once per tick, not per notification -- a handful of
        // notification-worthy lines a tick at most, so this is nowhere
        // near a hot path, but no reason to re-read the same small file
        // twice in a row either. Filtering (not the log line itself)
        // decides whether a *disabled* kind ever reaches the frontend at
        // all -- the backend owns "should this be audible right now",
        // the frontend just plays whatever it's handed.
        let settings = crate::settings::load(app);
        for n in notifications
            .into_iter()
            .filter(|n| settings.is_enabled(&n.kind))
        {
            let _ = app.emit("notification", n);
        }
    }
    for record in &finished {
        // Best-effort: a write failure here must not interrupt live
        // tailing. Nothing surfaces it today beyond this log line --
        // acceptable for now, but silent enough that it's worth
        // revisiting if history ever looks incomplete.
        if let Err(e) = crate::history::append(app, record) {
            eprintln!("parse history write failed for {}: {e}", record.target);
        }
    }
    // why: same best-effort stance as the history write just above -- a
    // failed profile save must not interrupt live tailing either. `st.
    // character` is `None` before the first zone/cast line has been seen
    // at all (see `identity_from_filename`'s own doc); nothing to key a
    // profile on yet in that window.
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
