//! why: what Debug > Parsed's search costs on a real log -- browse, a
//! no-match regex (full scan) and a common one, per table
//! input: path to a real log
//! run: cargo run -p eqlp-app --release --example dbsearch_cost -- <log>

use eqlp_app::dbsearch::{search, TABLES};
use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;
use std::time::Instant;

fn med(f: impl Fn()) -> f64 {
    let mut runs: Vec<u128> = (0..3)
        .map(|_| {
            let t = Instant::now(); // clock-exempt: benchmark measures wall time by definition
            f();
            t.elapsed().as_micros()
        })
        .collect();
    runs.sort_unstable();
    runs[1] as f64 / 1e3
}

fn main() {
    let path = std::env::args().nth(1).expect("usage: dbsearch_cost <log>");
    let raw = std::fs::read(&path).unwrap_or_else(|e| panic!("couldn't read {path}: {e}"));
    let lines = framed_lines(&raw);
    let engine = build_engine().expect("pack builds");
    let mut ing = Ingest::default();
    for chunk in lines.chunks(100_000) {
        backfill_lines(&mut ing, &engine, chunk, 8);
    }
    ing.mark_live();
    ing.tick(0);
    println!("{} lines, {} events", lines.len(), ing.store.len());
    println!(
        "{:<18} {:>9} {:>10} {:>10} {:>10}",
        "table", "rows", "browse ms", "nomatch ms", "common ms"
    );
    for t in TABLES {
        let total = search(&ing, t, "", 1).map(|d| d.total).unwrap_or(0);
        let browse = med(|| {
            let _ = search(&ing, t, "", 100);
        });
        let nomatch = med(|| {
            let _ = search(&ing, t, "zzqqxx", 100);
        });
        let common = med(|| {
            let _ = search(&ing, t, "a", 100);
        });
        println!("{t:<18} {total:>9} {browse:>10.1} {nomatch:>10.1} {common:>10.1}");
    }
}
