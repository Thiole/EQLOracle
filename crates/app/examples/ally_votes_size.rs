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
    let entries: usize = ing.ally_chains.values().map(|v| v.len()).sum();
    let names = ing.ally_chains.len();
    let votes: u32 = ing.ally_chains.values().flatten().map(|c| c.votes).sum();
    let bytes: usize = ing
        .ally_chains
        .iter()
        .map(|(k, v)| k.len() + v.iter().map(|c| 48 + c.scores.len() * 40).sum::<usize>())
        .sum();
    println!(
        "{} lines in {:.1}s; chains {entries} ({} distinct names), {votes} votes, ~{} KB",
        lines.len(),
        t.elapsed().as_secs_f64(),
        names,
        bytes / 1024
    );
}
