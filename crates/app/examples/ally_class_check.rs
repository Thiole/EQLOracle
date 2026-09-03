//! why: how good is ally class prediction? Every player with a /who row
//!      AND a predicted configuration: does the prediction fit the trio?
//! input: <log>
use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
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
    let (mut both, mut subset, mut wrong, mut none) = (0, 0, 0, 0);
    let mut examples: Vec<String> = Vec::new();
    for ((visit, name), (level, trio, _)) in &ing.who_seen {
        let (cfg, visits) = ing.ally_classes(name, *visit);
        if cfg.is_empty() {
            none += 1;
            continue;
        }
        both += 1;
        let fits = cfg.iter().all(|c| trio.contains(c));
        if fits {
            subset += 1;
        } else {
            wrong += 1;
            if examples.len() < 8 {
                examples.push(format!(
                    "  {name} ({level}) who={trio:?} inferred={cfg:?} ({visits} votes)"
                ));
            }
        }
    }
    println!("/who players: {}; with a prediction: {both}; prediction within the trio: {subset}; off: {wrong}; no prediction: {none}", ing.who_seen.len());
    for e in examples {
        println!("{e}");
    }
}
