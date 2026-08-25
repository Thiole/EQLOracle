//! why: what's still genuinely unmatched after the FULL real pipeline --
//!      rules, flavordata, third-person/verb-conjugated flavor,
//!      spelltext, and the effect-polarity fallback -- not just the raw
//!      regex engine (eqlp coverage doesn't see any of app's own
//!      fallback tiers, a different crate entirely).
//! input: path to a real log
//! output: top unmatched shapes by real line count, post-fallback
//! run: cargo run -p eqlp-app --release --example unmatched_shapes_check -- <log> [top-n]

use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;

const BACKFILL_CHUNK_LINES: usize = 100_000;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .expect("usage: unmatched_shapes_check <log> [top-n]");
    let top_n: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(40);

    let raw = std::fs::read(&path).unwrap_or_else(|e| panic!("couldn't read {path}: {e}"));
    let lines = framed_lines(&raw);
    let engine = build_engine().expect("pack builds");
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);

    let mut ing = Ingest::default();
    for chunk in lines.chunks(BACKFILL_CHUNK_LINES) {
        backfill_lines(&mut ing, &engine, chunk, threads);
    }

    let total = ing.counts.total;
    let matched = ing.counts.matched;
    let unmatched = ing.counts.unmatched;
    println!("total lines:   {total}");
    println!(
        "rule-matched:  {matched} ({:.2}%)",
        100.0 * matched as f64 / total as f64
    );
    println!(
        "unmatched:     {unmatched} ({:.2}%) -- after rules + flavordata + spelltext + polarity",
        100.0 * unmatched as f64 / total as f64
    );
    println!("distinct shapes: {}", ing.unmatched_shapes_distinct());
    println!(
        "overflow lines (beyond shape cap): {}",
        ing.unmatched_shapes_overflow()
    );

    println!("\ntop {top_n} still-unmatched shapes");
    for (shape, stat) in ing.unmatched_shapes_top(top_n) {
        println!("{:>10}  {}", stat.count, String::from_utf8_lossy(shape));
        println!(
            "            e.g. {}",
            String::from_utf8_lossy(&stat.example)
        );
    }
}
