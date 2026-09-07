//! why: "chain closed by a loadout swap signal -- why doesn't it show the
//! data it had at the time of that zone". For every closed ally chain,
//! what its frozen view still answers when asked at a unit inside it.
//! input: path to a real log
//! run: cargo run -p eqlp-app --release --example closed_chain_view -- <log>

use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: closed_chain_view <log>");
    let raw = std::fs::read(&path).unwrap_or_else(|e| panic!("couldn't read {path}: {e}"));
    let lines = framed_lines(&raw);
    let engine = build_engine().expect("pack builds");
    let threads = std::thread::available_parallelism()
        .map(|n| n.get().min(16))
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
    let you = ing.store.names.get("You").map(|s| s.0);
    let (mut closed, mut empty, mut swap, mut contra) = (0, 0, 0, 0);
    let mut samples: Vec<String> = Vec::new();
    for e in ing.classes.known_entities() {
        if Some(e) == you {
            continue;
        }
        let name = ing.store.name(eqlp_store::Sym(e)).to_string();
        for c in ing.classes.chains(e) {
            let Some(end) = c.closed else { continue };
            closed += 1;
            match end {
                eqlp_session::classdetect::ChainEnd::Swap => swap += 1,
                eqlp_session::classdetect::ChainEnd::Contradiction => contra += 1,
                eqlp_session::classdetect::ChainEnd::Presence => swap += 1,
            }
            // why: ask the way the ally table does -- at a unit the chain covered
            let at = ing.classes.chain_at(e, c.first);
            let shown = at.as_ref().map(|v| v.inferred()).unwrap_or_default();
            if shown.is_empty() {
                empty += 1;
                if samples.len() < 8 {
                    samples.push(format!(
                        "{name}: units {:?}..{:?} end={end:?} confirmed={:?} prior={:?} leading={:?} candidates={:?} evidence-units={}",
                        c.first, c.last, c.confirmed, c.prior, c.leading, c.candidates, c.units
                    ));
                }
            }
        }
    }
    println!("closed ally chains: {closed} (swap {swap}, contradiction {contra}); answering NOTHING at their own first unit: {empty}");
    for s in samples {
        println!("  {s}");
    }
}
