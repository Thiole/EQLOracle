//! why: real-data spot check for combat::class_configurations -- dumps
//! every resolved configuration's own level_range plus the unresolved
//! visit count, so a real "why didn't my level update" report can be
//! traced to an actual folding/evidence gap instead of guessed at.
//! input: path to a real log, character name
//! run: cargo run -p eqlp-app --release --example class_level_check -- <log> <name>

use eqlp_app::combat;
use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: class_level_check <log> <name>");
    let name = args.next().unwrap_or_else(|| "You".to_string());
    let raw = std::fs::read(&path).unwrap_or_else(|e| panic!("couldn't read {path}: {e}"));
    let lines = framed_lines(&raw);
    let engine = build_engine().expect("pack builds");
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);

    let mut ing = Ingest::default();
    for chunk in lines.chunks(100_000) {
        backfill_lines(&mut ing, &engine, chunk, threads);
    }
    ing.mark_live();
    ing.tick(0);

    let dto = combat::class_configurations(&ing, &name);
    println!("unresolved_visits: {}", dto.unresolved_visits);
    for c in &dto.configurations {
        println!(
            "{:>3} visits  {:?}  level_range={:?}",
            c.zone_visits, c.classes, c.level_range
        );
    }

    println!("\nlatest level: {:?}", ing.levels.latest());
    println!("latest level ts: {:?}", ing.levels.latest_ts());

    // why: which zone visit is currently open (the last one), and its
    // own start ts -- for cross-checking against the level.up ts above
    if let Some(i) = ing.zone.len().checked_sub(1) {
        if let Some((start, next)) = ing.zone.bounds(i) {
            println!("last zone visit index {i}: start={start} next={next:?}");
        }
    }
}
