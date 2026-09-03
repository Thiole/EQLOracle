//! why: how good is ally class inference? Every activity chain that has
//!      a /who row: does the chain's own inference fit the row's trio?
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
    let (mut rows, mut both, mut fits, mut wrong, mut none) = (0, 0, 0, 0, 0);
    let mut examples: Vec<String> = Vec::new();
    for (name, chains) in &ing.ally_chains {
        for chain in chains {
            let Some((level, trio)) = &chain.who else {
                continue;
            };
            rows += 1;
            let (cfg, votes) = ing.ally_classes(name, chain.start);
            // why: a prior-only chain (no fresh votes) is a "?" in the UI,
            // not an inference -- it doesn't count either way
            if cfg.is_empty() || votes == 0 {
                none += 1;
                continue;
            }
            both += 1;
            if cfg.iter().all(|c| trio.contains(c)) {
                fits += 1;
            } else {
                wrong += 1;
                if examples.len() < 8 {
                    examples.push(format!(
                        "  {name} ({level}) who={trio:?} inferred={cfg:?} ({votes} votes)"
                    ));
                }
            }
        }
    }
    println!(
        "/who chains: {rows}; with an inference: {both}; inside the trio: {fits}; off: {wrong}; no inference: {none}"
    );
    for e in examples {
        println!("{e}");
    }
}
