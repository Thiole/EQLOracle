//! why: what a read costs once the log is in -- the README's "every fight
//! costs the same" and "aggregates are queries" are claims, this is the
//! number. Full replay, then each of the UI's main reads timed cold and
//! then warm, against the whole log and against one fight.
//! input: path to a real log
//! run: cargo run -p eqlp-app --release --example query_cost -- <log>

use eqlp_app::combat;
use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;
use std::time::Instant;

fn timed<T>(label: &str, f: impl Fn() -> T) -> T {
    // why: first call pays any lazy init; the median of the rest is the
    // steady-state read
    let t0 = Instant::now();
    let out = f();
    let cold = t0.elapsed();
    let mut runs: Vec<u128> = (0..5)
        .map(|_| {
            let t = Instant::now();
            let _ = f();
            t.elapsed().as_micros()
        })
        .collect();
    runs.sort_unstable();
    println!(
        "{label:<44} cold {:>8.2} ms   warm {:>8.2} ms",
        cold.as_secs_f64() * 1e3,
        runs[runs.len() / 2] as f64 / 1e3
    );
    out
}

fn main() {
    let path = std::env::args().nth(1).expect("usage: query_cost <log>");
    let raw = std::fs::read(&path).unwrap_or_else(|e| panic!("couldn't read {path}: {e}"));
    let lines = framed_lines(&raw);
    let engine = build_engine().expect("pack builds");
    let threads = std::thread::available_parallelism()
        .map(|n| n.get().min(16))
        .unwrap_or(4);
    let t0 = Instant::now();
    let mut ing = Ingest::default();
    for chunk in lines.chunks(100_000) {
        backfill_lines(&mut ing, &engine, chunk, threads);
    }
    ing.mark_live();
    ing.tick(0);
    let backfill = t0.elapsed();
    println!(
        "backfill: {} lines in {:.2}s on {threads} threads ({:.0} lines/s), {} events, {} encounters",
        lines.len(),
        backfill.as_secs_f64(),
        lines.len() as f64 / backfill.as_secs_f64(),
        ing.store.len(),
        ing.store.encounters.len()
    );

    let encs = timed("list_encounters(all, 0..50)", || {
        combat::list_encounters(&ing, None, 0, 50)
    });
    let one = encs.first().map(|e| e.id);
    // why: the oldest fight by id, not by list offset -- the list is
    // sorted newest first and filtered, so its last page is not id 0
    let first = combat::list_encounters(&ing, None, 0, usize::MAX)
        .iter()
        .map(|e| e.id)
        .min();

    timed("summarize(all zones, aggregate)", || {
        combat::summarize(&ing, None, None, None, false)
    });
    timed("list_allies(all zones, aggregate)", || {
        combat::list_allies(&ing, None, None, false)
    });
    timed("list_enemies(all zones, aggregate)", || {
        combat::list_enemies(&ing, None, None, false)
    });
    if let Some(id) = one {
        timed("summarize(most recent fight)", || {
            combat::summarize(&ing, None, Some(id), None, false)
        });
        timed("list_allies(most recent fight)", || {
            combat::list_allies(&ing, None, Some(id), false)
        });
        timed("fight_timeline(most recent fight)", || {
            combat::fight_timeline(&ing, id)
        });
    }
    if let Some(id) = first {
        timed("summarize(oldest fight)", || {
            combat::summarize(&ing, None, Some(id), None, false)
        });
    }
    timed("live_meter (overlay poll)", || combat::live_meter(&ing));
}
