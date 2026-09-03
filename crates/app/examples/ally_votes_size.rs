//! why: how much the class-vote tables cost on a real log -- entries and bytes
use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;
fn main() {
    let raw = std::fs::read(std::env::args().nth(1).unwrap()).unwrap();
    let lines = framed_lines(&raw);
    let engine = build_engine().unwrap();
    let mut ing = Ingest::default();
    let t = std::time::Instant::now(); // clock-exempt: probe timing
    for chunk in lines.chunks(100_000) {
        backfill_lines(&mut ing, &engine, chunk, 8);
    }
    let entries = ing.ally_votes.len();
    let names: std::collections::HashSet<&String> = ing.ally_votes.keys().map(|(_, n)| n).collect();
    let votes: u32 = ing.ally_votes.values().map(|v| v.1).sum();
    let bytes: usize = ing.ally_votes.iter().map(|(k, v)| k.1.len() + 32 + v.0.len() * 40).sum();
    println!("{} lines in {:.1}s; vote entries {entries} ({} distinct names), {votes} votes, ~{} KB", lines.len(), t.elapsed().as_secs_f64(), names.len(), bytes / 1024);
}
