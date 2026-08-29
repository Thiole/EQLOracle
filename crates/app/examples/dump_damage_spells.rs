//! why: dump list_damage_spells as JSON after a real-log replay -- feeds
//! the frontend rotation-sim A/B harness so sim changes can be tested
//! against the exact candidate set the UI sees, not synthetic dtos.
//! run: cargo run -p eqlp-app --release --example dump_damage_spells -- <log>

use eqlp_app::dpscalc::list_damage_spells;
use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: dump_damage_spells <log>");
    let raw = std::fs::read(&path).expect("read log");
    let engine = build_engine().expect("pack builds");
    let lines = framed_lines(&raw);
    let mut ing = Ingest::default();
    for chunk in lines.chunks(100_000) {
        backfill_lines(&mut ing, &engine, chunk, 8);
    }
    let all = list_damage_spells(&ing, false);
    println!("{}", serde_json::to_string(&all).expect("serialize"));
}
