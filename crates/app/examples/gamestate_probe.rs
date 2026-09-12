//! why: the two new Game State columns against a real replay
//! input: <log>
use eqlp_app::debugview::game_state;
use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;

fn main() {
    let path = std::env::args().nth(1).expect("log path");
    let raw = std::fs::read(&path).unwrap();
    let lines = framed_lines(&raw);
    let engine = build_engine().expect("pack builds");
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let mut ing = Ingest::default();
    for chunk in lines.chunks(100_000) {
        backfill_lines(&mut ing, &engine, chunk, threads);
    }
    ing.tick(ing.now_ms());
    let gs = game_state(&ing);
    println!("YOUR SIDE ({} members)", gs.party.len());
    for p in &gs.party {
        println!("  {:<20} {}", p.name, p.via);
        for pet in &p.pets {
            println!("      {:<40} {}", pet.name, pet.kind);
        }
    }
    println!("\nSTILL ALIVE AGAINST YOU ({})", gs.enemies.len());
    for e in gs.enemies.iter().take(20) {
        println!("  {e}");
    }
}
