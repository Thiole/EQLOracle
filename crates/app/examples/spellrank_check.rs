//! why: verify progression::spell_ranks against a real replayed log, not
//!      just synthetic unit-test cases -- the whole feature depends on
//!      disambiguating live cast-rank suffixes ("Ice Comet X") from real
//!      spell-line catalog names ("Monster Summoning II") correctly
//!      across the *entire* real cast history, not a handful of examples.
//! input: path to a real log
//! output: printed base spell name -> highest observed rank, sorted by
//!         rank descending, plus a sanity spot-check on Ice Comet
//! run: cargo run -p eqlp-app --example spellrank_check -- <log>

use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;
use eqlp_app::progression::spell_ranks;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: spellrank_check <path-to-log>");
    let raw = std::fs::read(&path).unwrap_or_else(|e| panic!("couldn't read {path}: {e}"));
    let lines = framed_lines(&raw);
    let engine = build_engine().expect("pack builds");
    let mut ing = Ingest::default();
    for chunk in lines.chunks(100_000) {
        backfill_lines(&mut ing, &engine, chunk, 8);
    }

    let mut ranks: Vec<(String, u8)> = spell_ranks(&ing).into_iter().collect();
    ranks.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

    println!(
        "{} spells with an observed live rank this replay:",
        ranks.len()
    );
    for (name, rank) in &ranks {
        println!("  {name:<40} rank {rank}");
    }

    // why: the exact example from the conversation -- must show up, and
    // must NOT accidentally show "Monster Summoning"/"Monster Summoning
    // II" as if II/III were ranks of a base "Monster Summoning".
    match ranks.iter().find(|(n, _)| n == "Ice Comet") {
        Some((_, r)) => println!("\nOK: Ice Comet observed at rank {r}"),
        None => println!("\nWARN: Ice Comet not observed at any rank"),
    }
    for bad in ["Monster Summoning", "Yaulp"] {
        if ranks.iter().any(|(n, _)| n == bad) {
            println!("BUG: {bad:?} showed up as a base spell with an observed rank -- it should never split");
        }
    }
}
