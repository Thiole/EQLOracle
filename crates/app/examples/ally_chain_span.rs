//! why: "class detection of allies ... should only use current zone
//! visit" -- for every ally with a chain, how many of their chains span
//! more than one unit. A chain that crosses a zone visit is evidence
//! carried across it.
//! input: path to a real log
//! run: cargo run -p eqlp-app --release --example ally_chain_span -- <log>

use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: ally_chain_span <log>");
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
    // why: your zone entries as units -- a chain whose range contains one
    // strictly after its first unit carried evidence across that zone line
    let zone_units: Vec<usize> = ing
        .zone
        .iter()
        .filter_map(|(t, _)| ing.unit_at(t))
        .collect();
    let mut crossing = 0;
    let mut crossing_names: std::collections::BTreeMap<String, usize> = Default::default();
    let (mut entities, mut chains, mut multi, mut max_units) = (0, 0, 0, 0usize);
    let mut worst: Vec<(usize, String, usize)> = Vec::new();
    for e in ing.classes.known_entities() {
        if Some(e) == you {
            continue;
        }
        let cs = ing.classes.chains(e);
        if cs.is_empty() {
            continue;
        }
        entities += 1;
        chains += cs.len();
        let name = ing.store.name(eqlp_store::Sym(e)).to_string();
        for c in &cs {
            let span = match (c.first, c.last) {
                (Some(a), Some(b)) if b >= a => b - a + 1,
                _ => 1,
            };
            if span > 1 {
                multi += 1;
            }
            if let (Some(a), Some(b)) = (c.first, c.last) {
                if zone_units.iter().any(|&z| z > a && z <= b) {
                    crossing += 1;
                    *crossing_names.entry(name.clone()).or_default() += 1;
                }
            }
            max_units = max_units.max(span);
            worst.push((span, name.clone(), c.units));
        }
    }
    worst.sort_by_key(|w| std::cmp::Reverse(w.0));
    println!("allies with chains: {entities}, chains: {chains}, chains spanning >1 unit: {multi}, widest span: {max_units} units");
    println!(
        "your zone entries: {}, ally chains that CROSS one: {crossing}",
        zone_units.len()
    );
    let mut top: Vec<_> = crossing_names.into_iter().collect();
    top.sort_by_key(|t| std::cmp::Reverse(t.1));
    for (n, k) in top.iter().take(8) {
        println!("  {n:<18} {k} crossing chains");
    }
    for (span, name, units) in worst.iter().take(12) {
        println!("  {name:<18} spans {span} units, evidence in {units}");
    }
}
