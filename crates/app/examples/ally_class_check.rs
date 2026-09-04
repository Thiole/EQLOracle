//! why: how good is class inference on its own? Every chain that has a
//!      /who row is ground truth: does the chain's own inference, which
//!      never sees that row, fit the trio it states?
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
    let entities: Vec<u32> = ing.classes.known_entities().collect();
    for sym in entities {
        let name = ing.store.name(eqlp_store::Sym(sym)).to_string();
        for chain in ing.classes.chains(sym) {
            let Some((level, trio)) = chain.who.clone() else {
                continue;
            };
            rows += 1;
            let cfg = chain.inferred();
            if cfg.is_empty() {
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
                        "  {name} ({level}) who={trio:?} inferred={cfg:?} ({} encounters)",
                        chain.units
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
