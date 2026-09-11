//! why: who ends up in the ally table over a whole log, and via which
//! channel -- Kind::Player bypasses GroupTracker in allegiance_at
//! input: <log>
use eqlp_app::combat::list_allies;
use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;

fn main() {
    let path = std::env::args().nth(1).expect("log path");
    let raw = std::fs::read(&path).unwrap_or_else(|e| panic!("couldn't read {path}: {e}"));
    let lines = framed_lines(&raw);
    let engine = build_engine().expect("pack builds");
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let mut ing = Ingest::default();
    for chunk in lines.chunks(100_000) {
        backfill_lines(&mut ing, &engine, chunk, threads);
    }
    ing.tick(ing.now_ms());
    let now = ing.now_ms();

    let allies = list_allies(&ing, None, None, false, None);
    println!("ally rows over the WHOLE history: {}", allies.len());
    let grouped = allies
        .iter()
        .filter(|a| ing.groups.currently_grouped(&a.name, now))
        .count();
    println!("  of those, currently_grouped(now): {grouped}");
    println!(
        "  so {} are allies WITHOUT group evidence",
        allies.len() - grouped
    );
    println!("\nrows carrying pets, plus Sidhe:");
    for a in allies
        .iter()
        .filter(|a| !a.pets.is_empty() || a.name == "Sidhe")
    {
        let ev = ing.groups.evidence_for(&a.name);
        println!(
            "  {:<22} {:>12}  pets={:<3} sessions={:?}",
            a.name,
            a.total,
            a.pets.len(),
            ev.map(|e| e.1)
        );
    }
}
