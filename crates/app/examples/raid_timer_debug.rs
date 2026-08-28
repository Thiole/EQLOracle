//! why: dump every zone visit matching a given wiki zone name, with its
//! exact raw label, span, and whatever first-pull/boss-kill this visit
//! would compute -- to debug why some real solo clears aren't producing
//! a fastest-time entry.
//! run: cargo run -p eqlp-app --release --example raid_timer_debug -- <log> <wiki-zone-name> <boss-log-name>

use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;
use eqlp_app::raiding::debug_visit_trace;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: raid_timer_debug <log> <wiki-zone-name> <boss-log-name>");
    let zone_name = std::env::args().nth(2).expect("need a wiki zone name");
    let boss_log_name = std::env::args().nth(3).expect("need a boss log name");
    let raw = std::fs::read(&path).unwrap_or_else(|e| panic!("couldn't read {path}: {e}"));
    let lines = framed_lines(&raw);
    let engine = build_engine().expect("pack builds");
    let mut ing = Ingest::default();
    for chunk in lines.chunks(100_000) {
        backfill_lines(&mut ing, &engine, chunk, 8);
    }

    for line in debug_visit_trace(&ing, &zone_name, &boss_log_name) {
        println!("{line}");
    }
}
