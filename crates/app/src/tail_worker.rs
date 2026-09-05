//! The live loop: which file is the game writing to right now, and what
//! does it say. One background thread per configured directory.
//!
//! Design notes: `docs/design/sources.md` -- polling (not filesystem
//! notification), "newest file in the folder" is the only reliable signal
//! for which character is currently logged in, and a `Framer` is required
//! while tailing because a poll can land mid-line.

use crate::ingest::{self, Ingest, LineCounts, RecentLine};
use crate::state::LockRecover;

/// why: run one backfill batch so a single panicking parse line can't
/// poison the ingest mutex (which every command locks) and brick the
/// app. A panic leaves Ingest memory-safe, only mid-update -- we log,
/// drop the batch, and keep tailing; the next full replay on restart
/// rebuilds clean. AssertUnwindSafe: a &mut across catch_unwind is not
/// UnwindSafe by default, but the recovery contract above accepts a
/// logically-partial Ingest, so the assertion is sound here.
fn backfill_guarded(
    ing: &mut crate::ingest::Ingest,
    engine: &eqlp_core::Engine,
    refs: &[&[u8]],
    threads: usize,
) {
    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ingest::backfill_lines(ing, engine, refs, threads);
    }));
    if res.is_err() {
        eprintln!(
            "[eqlp] a parse batch panicked and was skipped ({} lines) -- app stays live",
            refs.len()
        );
    }
}
use crate::inventory::{inventory_character, is_inventory_dump};
use eqlp_core::frame::Framer;
use eqlp_source::{identity_from_filename, newest_log_in, SystemClock, Tail, TailEvent, POLL_MS};
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

/// why: how much of a log's tail makes the app USABLE, measured rather
/// than guessed (examples/backfill_window.rs, real 378 MiB log): the
/// current zone reads correctly off the last 4 MiB, and 19 MiB folds in
/// 796 ms against 16.4 s for the whole file. Everything the launch screen
/// answers -- where am I, what am I fighting, what did I just loot --
/// lives in the tail. Cumulative history (kills ever, loot ever,
/// progression) does not, which is what the second pass is for.
const WARM_START_BYTES: u64 = 24 * 1024 * 1024;

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

/// why: a byte offset into a log is only meaningful at a line boundary --
/// starting mid-line hands the framer a fragment that parses as nothing,
/// or worse, as something else. Walks FORWARD to the next newline, so the
/// warm pass starts on a whole line and never re-reads a partial one.
fn line_aligned_offset(path: &Path, want: u64) -> u64 {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(path) else {
        return 0;
    };
    if f.seek(SeekFrom::Start(want)).is_err() {
        return 0;
    }
    let mut buf = vec![0u8; 256 * 1024];
    let Ok(n) = f.read(&mut buf) else {
        return 0;
    };
    match buf[..n].iter().position(|&b| b == b'\n') {
        Some(i) => want + i as u64 + 1,
        // why: no newline in a whole buffer -- give up on the warm start
        // rather than guess; the full fold still runs
        None => 0,
    }
}

/// why: the SECOND pass -- the whole log, folded into a fresh `Ingest`
/// behind the warm one, then swapped in by the caller. Deliberately not a
/// thread: nothing else needs the CPU while it runs, and a swap between
/// two threads is a consistency problem this does not have to own. The
/// tail and framer come back with it so the live seam continues from the
/// byte this fold actually reached.
fn fold_history(
    path: &Path,
    engine: &eqlp_core::Engine,
    log_dir: &Path,
    threads: usize,
    stop: &AtomicBool,
    mut progress: impl FnMut(),
) -> Option<(Ingest, Tail, Framer)> {
    let mut ing = Ingest::default();
    if let Some(base) = log_dir.parent() {
        ing.set_spell_file(base);
    }
    ing.character = identity_from_filename(path).map(|(c, _)| c);
    let mut t = Tail::from_start(path);
    let mut framer = Framer::default();
    let mut batch: Vec<Vec<u8>> = Vec::with_capacity(BACKFILL_CHUNK_LINES);
    let mut aborted = false;
    // why: same streamed, bounded-batch shape as the warm pass -- a slurp
    // put peak memory near 1 GiB on a 322 MB log and got the process killed
    t.poll(|chunk| {
        if aborted {
            return;
        }
        framer.push(chunk, |line| batch.push(line.to_vec()));
        while batch.len() >= BACKFILL_CHUNK_LINES {
            if stop.load(Ordering::Relaxed) {
                aborted = true;
                batch.clear();
                return;
            }
            let refs: Vec<&[u8]> = batch.iter().map(|v| v.as_slice()).collect();
            backfill_guarded(&mut ing, engine, &refs, threads);
            batch.clear();
            progress();
        }
    });
    if aborted {
        return None;
    }
    if !batch.is_empty() {
        let refs: Vec<&[u8]> = batch.iter().map(|v| v.as_slice()).collect();
        backfill_guarded(&mut ing, engine, &refs, threads);
    }
    Some((ing, t, framer))
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
    // why: backfill parses in parallel, live growth stays single-threaded;
    // 16-thread cap re-measured via examples/backfill_bench.rs.
    //
    // Reverted a real attempt at 2x-oversubscribing this (see git history):
    // backfill_bench.rs's own sweep on a dev machine said more threads
    // keeps winning past available_parallelism(), but reported live on a
    // real machine, startup got slower, not faster, with more threads.
    // The bench only measures raw parse throughput in isolation --
    // backfill on a real machine can run while the game itself is live
    // (reconnect mid-session, app restart), and there it's competing with
    // the actual game client for the same cores. Oversubscribing wins the
    // synthetic benchmark and loses the real machine. Raw detected count,
    // not multiplied.
    let backfill_threads = thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(16);

    // why: fresh directory starts fresh -- old row/encounter ids mean nothing here
    {
        // why: L8 -- spells_us.txt sits beside the Logs folder
        let mut fresh = Ingest::default();
        if let Some(base) = log_dir.parent() {
            fresh.set_spell_file(base);
        }
        *ingest.lock_recover() = fresh;
    }

    let mut target: Option<PathBuf> = None;
    let mut tail: Option<Tail> = None;
    let mut framer = Framer::default();
    let mut backfilling = false;
    // why: set when the warm pass skipped history, cleared when the full
    // fold has replaced it -- Some(path) is "this log still owes a re-fold"
    let mut history_pending: Option<PathBuf> = None;

    // why: EQLP_REPLAY_UNTIL="Wed Sep 02 11:30:00 2026" -- replay the log
    // up to that instant and freeze there, mid-fight if that is where it
    // lands. Every module then shows the real state of that moment: the
    // party you were in, their detected classes, the buffs actually up,
    // the meter mid-encounter. The clock never advances past it, so
    // nothing decays or closes behind your back.
    let replay_until: Option<i64> = std::env::var("EQLP_REPLAY_UNTIL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .and_then(|s| {
            let stamp = format!("[{}] ", s.trim());
            eqlp_core::header::by_name("bracket-ctime")
                .and_then(|h| h.parse(stamp.as_bytes()))
                .map(|(ts, _)| ts.secs() * 1000)
        });
    let mut frozen = false;

    let mut last_rescan = clock.now_ms() - RESCAN_MS;
    let mut last_emit = clock.now_ms() - HEARTBEAT_MS;

    // why: how much of a batch is at or before the freeze point
    let keep_upto = |batch: &[Vec<u8>], until: i64| -> usize {
        let h = eqlp_core::header::by_name("bracket-ctime");
        batch
            .iter()
            .position(|l| {
                h.as_ref()
                    .and_then(|h| h.parse(l))
                    .is_some_and(|(ts, _)| ts.secs() * 1000 > until)
            })
            .unwrap_or(batch.len())
    };

    while !stop.load(Ordering::Relaxed) && !frozen {
        // why: the warm pass is live and serving by now, so the full fold
        // costs the user nothing but a "still loading history" label. Runs
        // before the rescan so a freshly switched log warms first.
        if !backfilling {
            if let Some(path) = history_pending.take() {
                emit_tick(&app, &log_dir, &target, "history", true, &ingest, &status);
                let folded =
                    fold_history(&path, &engine, &log_dir, backfill_threads, &stop, || {
                        emit_tick(&app, &log_dir, &target, "history", true, &ingest, &status)
                    });
                if let Some((full, full_tail, full_framer)) = folded {
                    {
                        let mut ing = ingest.lock_recover();
                        *ing = full;
                        ing.mark_live();
                        ing.tick(clock.now_ms());
                    }
                    // why: the seam continues from where the FULL fold
                    // reached, not where the warm one did -- the warm tail
                    // and its framer are discarded with the state they built
                    tail = Some(full_tail);
                    framer = full_framer;
                    emit_tick(&app, &log_dir, &target, "grew", false, &ingest, &status);
                }
            }
        }

        let now = clock.now_ms();
        let mut switched = false;

        if now - last_rescan >= RESCAN_MS {
            last_rescan = now;
            if let Some(newest) = newest_log_in(&log_dir) {
                if target.as_ref() != Some(&newest) {
                    // why: whose log this is -- read before the move
                    let character = identity_from_filename(&newest).map(|(c, _)| c);
                    target = Some(newest.clone());
                    // why: WARM START -- fold the tail first so the app is
                    // usable in about a second, then re-fold the whole file
                    // behind it (see `history_pending`). The full history is
                    // never skipped, only deferred: a launch that used to
                    // show nothing for twenty seconds now shows the right
                    // zone, fight and loot almost immediately.
                    let len = std::fs::metadata(&newest).map(|m| m.len()).unwrap_or(0);
                    // why: EQLP_REPLAY_UNTIL freezes at an instant that can
                    // sit BEFORE a warm start would begin -- a probe asking
                    // for a moment in last week's play must replay the whole
                    // log to reach it, so the warm path is off there
                    let warm = if replay_until.is_none() && len > WARM_START_BYTES {
                        line_aligned_offset(&newest, len - WARM_START_BYTES)
                    } else {
                        0
                    };
                    history_pending = (warm > 0).then(|| newest.clone());
                    tail = Some(if warm > 0 {
                        Tail::from_offset(newest, warm)
                    } else {
                        // why: "watch live" and "browse past fights" are the
                        // same tail, just before/after catching up
                        Tail::from_start(newest)
                    });
                    framer = Framer::default();
                    backfilling = true;
                    let mut fresh = Ingest::default();
                    if let Some(base) = log_dir.parent() {
                        fresh.set_spell_file(base);
                    }
                    // why: whose log this is -- your own /who row then
                    // lands on "You" like every other self observation
                    fresh.character = character;
                    *ingest.lock_recover() = fresh;
                    switched = true;
                }
            }
        }

        let mut tail_status: &'static str = "idle";
        if let Some(t) = tail.as_mut() {
            if backfilling {
                // why: STREAMED, not slurped -- real incident: buffering
                // the whole file plus a full line index put backfill's
                // transient peak at ~1GiB on a 322MB log, and desktop
                // launches (systemd user units) were getting SIGKILLed at
                // almost exactly 1G by a host memory policy -- reproduced
                // three times, then proven by the same binary surviving
                // past 1.06G under an explicit MemoryMax=6G unit. Bounded
                // batches of owned lines (~100k at a time, processed and
                // dropped inside the poll sink) keep peak memory flat no
                // matter how big the log grows. Bonus: the trailing
                // partial line now stays buffered in the Framer and
                // continues seamlessly into the live path, instead of
                // being dropped for a possibly-misframed seam line.
                let mut batch: Vec<Vec<u8>> = Vec::with_capacity(BACKFILL_CHUNK_LINES);
                let mut aborted = false;
                let ev = t.poll(|chunk| {
                    if aborted {
                        return; // why: stop requested mid-replay, drain the rest unprocessed
                    }
                    framer.push(chunk, |line| batch.push(line.to_vec()));
                    while batch.len() >= BACKFILL_CHUNK_LINES {
                        if stop.load(Ordering::Relaxed) {
                            aborted = true;
                            batch.clear();
                            return;
                        }
                        let cut = replay_until.map_or(batch.len(), |u| keep_upto(&batch, u));
                        let refs: Vec<&[u8]> = batch[..cut].iter().map(|v| v.as_slice()).collect();
                        {
                            let mut ing = ingest.lock_recover();
                            backfill_guarded(&mut ing, &engine, &refs, backfill_threads);
                        }
                        let hit_freeze = cut < batch.len();
                        batch.clear();
                        if hit_freeze {
                            // why: EQLP_REPLAY_UNTIL -- stop here, and stop reading
                            aborted = true;
                            frozen = true;
                            return;
                        }
                        // why: unconditional -- the counting-up progress the UI shows
                        last_emit = SystemClock.now_ms();
                        emit_tick(&app, &log_dir, &target, "grew", true, &ingest, &status);
                    }
                });
                tail_status = tail_event_str(ev);
                if frozen {
                    // why: EQLP_REPLAY_UNTIL -- one last emit so the UI
                    // shows the frozen moment; without it the window kept
                    // whatever mid-replay state it last drew
                    emit_tick(&app, &log_dir, &target, "frozen", false, &ingest, &status);
                    return;
                }
                if aborted || stop.load(Ordering::Relaxed) {
                    return; // directory changed, or the app is closing, mid-replay
                }
                // why: whatever's left under one full batch
                if !batch.is_empty() {
                    let cut = replay_until.map_or(batch.len(), |u| keep_upto(&batch, u));
                    frozen |= cut < batch.len();
                    let refs: Vec<&[u8]> = batch[..cut].iter().map(|v| v.as_slice()).collect();
                    let mut ing = ingest.lock_recover();
                    backfill_guarded(&mut ing, &engine, &refs, backfill_threads);
                    drop(ing);
                    last_emit = clock.now_ms();
                    emit_tick(&app, &log_dir, &target, tail_status, true, &ingest, &status);
                }

                if frozen {
                    // why: EQLP_REPLAY_UNTIL -- the snapshot IS the state;
                    // going live would let the clock run past it and close
                    // the very fight the freeze was aimed at
                    emit_tick(&app, &log_dir, &target, "frozen", false, &ingest, &status);
                    return;
                }
                {
                    let mut ing = ingest.lock_recover();
                    // why: same panic-isolation as the batch path -- a
                    // bad line at the live seam must not kill the worker
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        ing.mark_live();
                        // why: a FRESH read, not the `now` from the top of
                        // this loop pass -- the batch above can take
                        // seconds, and a stale baseline here made the very
                        // next tick add that whole gap to the log clock, which
                        // then ran that far ahead of real time for the rest
                        // of the session (12s measured live): every fight
                        // closed the instant its kill line arrived
                        ing.tick(clock.now_ms());
                    }));
                }
                backfilling = false;
            } else {
                let mut ing = ingest.lock_recover();
                // why: catch a panic from any single live line (route ->
                // apply's indexing/arith) so the worker keeps tailing
                // and the mutex never poisons -- see backfill_guarded
                let ev = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let ev = t.poll(|chunk| {
                        framer.push(chunk, |line| {
                            let outcome = matcher.classify(line);
                            ing.route(&engine, line, &outcome);
                        });
                    });
                    ing.tick(clock.now_ms());
                    ev
                }))
                .unwrap_or_else(|_| {
                    eprintln!(
                        "[eqlp] a live parse line panicked and was skipped -- app stays live"
                    );
                    TailEvent::Idle
                });
                tail_status = tail_event_str(ev);
                // why: file changed identity mid-tail -- only the frame
                // boundary resets (possibly one misframed seam line), not
                // the parsed history; rebuilding it would be the wrong trade
                if matches!(ev, TailEvent::Truncated | TailEvent::Replaced) {
                    framer = Framer::default();
                }
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
    let pets_attributed = ingest.lock_recover().pet_owner_count();
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
    *status.lock_recover() = st.clone();

    let mut ing = ingest.lock_recover();
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
