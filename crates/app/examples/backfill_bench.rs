//! why: measures real startup backfill time, thread-count sweep
//! input: path to a real log
//! output: printed timings per thread count
//! run: cargo run -p eqlp-app --release --example backfill_bench -- <log>

use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;
use std::time::Instant;

const BACKFILL_CHUNK_LINES: usize = 100_000;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: backfill_bench <path-to-log>");
    let raw = std::fs::read(&path).unwrap_or_else(|e| panic!("couldn't read {path}: {e}"));
    println!("log size: {:.1} MiB", raw.len() as f64 / 1024.0 / 1024.0);

    let t_frame = Instant::now(); // clock-exempt: benchmark, measures real wall time on purpose
    let lines = framed_lines(&raw);
    println!("frame: {:?} ({} lines)", t_frame.elapsed(), lines.len());

    let t_engine = Instant::now(); // clock-exempt: benchmark, measures real wall time on purpose
    let engine = build_engine().expect("pack builds");
    println!(
        "engine build (rule pack + regex compile): {:?}",
        t_engine.elapsed()
    );

    let available = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    println!("available_parallelism: {available}");

    // why: sweep beyond the production cap to check it's still right
    let mut candidates: Vec<usize> = vec![1, 2, 4, 8, 12, 16, 24, 32];
    candidates.retain(|&t| t <= available * 2);
    candidates.dedup();

    for threads in candidates {
        let mut ing = Ingest::default();
        let t_backfill = Instant::now(); // clock-exempt: benchmark, measures real wall time on purpose
        let mut chunk_count = 0;
        for chunk in lines.chunks(BACKFILL_CHUNK_LINES) {
            backfill_lines(&mut ing, &engine, chunk, threads);
            chunk_count += 1;
        }
        let backfill_elapsed = t_backfill.elapsed();
        println!(
            "threads={threads:>2}  backfill={backfill_elapsed:>9.2?}  ({:.1} ns/line, {chunk_count} chunks)  matched={} unmatched={}",
            backfill_elapsed.as_nanos() as f64 / lines.len() as f64,
            ing.counts.matched,
            ing.counts.unmatched
        );
    }
}
