//! why: hp_rows sums every row targeting a NAME, but same-named mobs share
//! one Sym -- an AoE that hits N of them inflates one mob's HP N-fold
//! input: <log>
use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;
use eqlp_store::EventKind;
use std::collections::{HashMap, HashSet};

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

    // per encounter+target: summed vs AoE-deduped
    let mut raw_t: HashMap<(u32, u32), u64> = HashMap::new();
    let mut ded_t: HashMap<(u32, u32), u64> = HashMap::new();
    let mut seen: HashSet<(u32, u32, i64, u32)> = HashSet::new();
    for i in 0..ing.store.len() {
        if ing.store.kind[i] != EventKind::Damage {
            continue;
        }
        let k = (ing.store.enc[i], ing.store.target[i].0);
        *raw_t.entry(k).or_default() += ing.store.amount[i];
        // why: one (second, ability, target) group is ONE cast landing on
        // N siblings -- only one row is the mob being measured
        let g = (
            ing.store.enc[i],
            ing.store.target[i].0,
            ing.store.ts[i],
            ing.store.ability[i].0,
        );
        if seen.insert(g) {
            *ded_t.entry(k).or_default() += ing.store.amount[i];
        }
    }
    let mut worst: Vec<(f64, u64, u64, String)> = raw_t
        .iter()
        .filter_map(|(k, &r)| {
            let d = *ded_t.get(k)?;
            (d > 0 && r > d).then(|| {
                (
                    r as f64 / d as f64,
                    r,
                    d,
                    ing.store.names.name(eqlp_store::Sym(k.1)).to_string(),
                )
            })
        })
        .collect();
    worst.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
    let infl = raw_t.values().sum::<u64>() as f64 / ded_t.values().sum::<u64>() as f64;
    println!(
        "encounter/target pairs inflated by AoE grouping: {}",
        worst.len()
    );
    println!("overall summed/deduped ratio: {infl:.3}x\n");
    println!("worst offenders (ratio, summed, deduped, name):");
    for (r, a, b, n) in worst.iter().take(12) {
        println!("  {r:5.2}x  {a:>9}  {b:>9}  {n}");
    }
}
