use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::monsters::mob_stats;
use eqlp_app::parser::build_engine;
fn main() {
    let path = std::env::args().nth(1).unwrap();
    let raw = std::fs::read(&path).unwrap();
    let lines = framed_lines(&raw);
    let engine = build_engine().unwrap();
    let mut ing = Ingest::default();
    for chunk in lines.chunks(100_000) {
        backfill_lines(&mut ing, &engine, chunk, 8);
    }
    for name in ["a dracoliche", "a dracoliche pet", "Fright", "Cazic-Thule"] {
        let s = mob_stats(&ing, name);
        println!("{name}: kills={} pulls={}", s.kills, s.pulls);
    }
}
