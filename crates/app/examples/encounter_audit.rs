//! why: checks no real encounter spans more than one zone visit
//! input: path to a real log
//! output: longest 10 durations, zone-spanning count
//! run: cargo run -p eqlp-app --release --example encounter_audit -- <log>

use eqlp_app::combat;
use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: encounter_audit <path-to-log>");
    let raw = std::fs::read(&path).unwrap_or_else(|e| panic!("couldn't read {path}: {e}"));

    let engine = build_engine().expect("pack builds");
    let mut ing = Ingest::default();
    let lines = framed_lines(&raw);
    backfill_lines(&mut ing, &engine, &lines, 8);

    let mut encounters = combat::list_encounters(&ing, None, 0, usize::MAX);
    encounters.sort_by_key(|b| std::cmp::Reverse(b.duration_ms));

    println!(
        "{} real encounters, longest 10 by duration:",
        encounters.len()
    );
    let mut spanning = 0usize;
    for e in &encounters {
        let start_zone = ing.zone.index_at(e.start_ms);
        let end_zone = ing.zone.index_at(e.end_ms.unwrap_or(e.start_ms));
        if start_zone != end_zone {
            spanning += 1;
        }
    }
    for e in encounters.iter().take(10) {
        let start_zone = ing.zone.index_at(e.start_ms);
        let end_zone = ing.zone.index_at(e.end_ms.unwrap_or(e.start_ms));
        let flag = if start_zone != end_zone {
            "  <-- SPANS ZONES"
        } else {
            ""
        };
        println!(
            "  {:>8}ms  {:<30} slain={:<5} zone[start={:?} end={:?}]{}",
            e.duration_ms, e.target, e.slain, start_zone, end_zone, flag
        );
    }
    println!("encounters spanning more than one zone visit: {spanning} (should be 0)");
}
