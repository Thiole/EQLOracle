//! why: the fold is ~18s of a 18.4s backfill and cannot parallelize in file
//! order -- this says WHERE that time goes, per rule kind, which is what
//! decides whether splitting the fold by domain would buy anything.
//! input: path to a real log
//! output: per-kind line count and fold cost, plus the whole-log baseline
//! run: cargo run -p eqlp-app --release --example fold_cost -- <log>
use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;
use eqlp_core::Outcome;
use std::collections::HashMap;
use std::time::Instant;

fn fold_time(engine: &eqlp_core::Engine, lines: &[&[u8]]) -> std::time::Duration {
    let mut ing = Ingest::default();
    let t = Instant::now(); // clock-exempt: benchmark
    for chunk in lines.chunks(100_000) {
        backfill_lines(&mut ing, engine, chunk, 24);
    }
    t.elapsed()
}

fn main() {
    let path = std::env::args().nth(1).expect("usage: fold_cost <log>");
    let raw = std::fs::read(&path).unwrap_or_else(|e| panic!("couldn't read {path}: {e}"));
    let lines = framed_lines(&raw);
    let engine = build_engine().expect("pack builds");

    // why: one classification pass to bucket the log by the kind that claimed
    // each line -- the fold cost is then measured on each bucket separately
    let mut m = engine.matcher();
    let mut by_kind: HashMap<&str, Vec<&[u8]>> = HashMap::new();
    for line in &lines {
        if let Outcome::Matched(hit) = m.classify(line) {
            let kind = engine.rule(hit.rule).kind.as_str();
            by_kind.entry(kind).or_default().push(line);
        }
    }

    let whole = fold_time(&engine, &lines);
    println!("whole log: {:?} over {} lines\n", whole, lines.len());

    let mut rows: Vec<(&str, usize, std::time::Duration)> = by_kind
        .iter()
        .map(|(k, ls)| (*k, ls.len(), fold_time(&engine, ls)))
        .collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.2));
    println!(
        "{:<12} {:>10} {:>10} {:>8} {:>10}",
        "kind", "lines", "fold", "% whole", "ns/line"
    );
    let mut sum = std::time::Duration::ZERO;
    for (k, n, d) in &rows {
        sum += *d;
        println!(
            "{:<12} {:>10} {:>10.2?} {:>7.1}% {:>10.0}",
            k,
            n,
            d,
            100.0 * d.as_secs_f64() / whole.as_secs_f64(),
            d.as_nanos() as f64 / *n as f64
        );
    }
    println!(
        "\nsum of parts: {:?}  vs whole: {:?}  ({:.2}x)",
        sum,
        whole,
        sum.as_secs_f64() / whole.as_secs_f64()
    );
}
