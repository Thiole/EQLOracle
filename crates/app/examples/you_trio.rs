//! why: what the ally table actually shows for you at the end of a log --
//! the reported "still says Bard" is about this row, not about history.
//! input: path to a real log, optional name (default You)
//! run: cargo run -p eqlp-app --release --example you_trio -- <log> [name]

use eqlp_app::combat;
use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: you_trio <log> [name]");
    let who = std::env::args().nth(2).unwrap_or_else(|| "You".to_string());
    let raw = std::fs::read(&path).unwrap_or_else(|e| panic!("couldn't read {path}: {e}"));
    let lines = framed_lines(&raw);
    let engine = build_engine().expect("pack builds");
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let mut ing = Ingest::default();
    if let Some(base) = std::path::Path::new(&path)
        .parent()
        .and_then(|p| p.parent())
    {
        ing.set_spell_file(base);
    }
    for chunk in lines.chunks(100_000) {
        backfill_lines(&mut ing, &engine, chunk, threads);
    }
    ing.mark_live();
    ing.tick(0);

    for a in combat::list_allies(&ing, None, None, false, None) {
        if a.name == who {
            println!(
                "{who}: classes={:?} confirmed={} prior={:?} candidates={:?} source={} chain_end={:?} evidence={}",
                a.classes, a.class_confirmed, a.class_prior, a.class_candidates,
                a.class_source, a.class_chain_end, a.class_evidence
            );
        }
    }
    if let Some(sym) = ing.store.names.get(&who) {
        println!("chains: {}", ing.classes.chain_count(sym.0));
        for (cfg, n) in ing.classes.configurations_of(sym.0) {
            println!("  {n:>4} visits  {cfg:?}");
        }
    }
}
