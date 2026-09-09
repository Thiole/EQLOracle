//! why: rules C1-C4 on a real log -- what landed on each "(charmed)"
//!      instance, what stayed in its pool, and how much was unresolved
//! input: <log>
use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;
use eqlp_store::{EventKind, Sym};
use std::collections::HashMap;

fn main() {
    let path = std::env::args().nth(1).expect("log path");
    let raw = std::fs::read(&path).unwrap_or_else(|e| panic!("couldn't read {path}: {e}"));
    let lines = framed_lines(&raw);
    let engine = build_engine().expect("pack builds");
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let mut ing = Ingest::default();
    for chunk in lines.chunks(100_000) {
        backfill_lines(&mut ing, &engine, chunk, threads);
    }
    ing.mark_live();
    ing.tick(0);
    let mut dealt: HashMap<Sym, u64> = HashMap::new();
    let mut taken: HashMap<Sym, u64> = HashMap::new();
    let mut unresolved = 0u64;
    for i in 0..ing.store.len() {
        if ing.store.kind[i] != EventKind::Damage {
            continue;
        }
        if ing.store.flags[i] & eqlp_store::flag::UNRESOLVED_INSTANCE != 0 {
            unresolved += 1;
        }
        *dealt.entry(ing.store.actor[i]).or_default() += ing.store.amount[i];
        *taken.entry(ing.store.target[i]).or_default() += ing.store.amount[i];
    }
    let mut rows: Vec<(String, u64, u64, u64, u64)> = (0..ing.store.names.len())
        .map(|i| Sym(i as u32))
        .map(|s| (ing.store.names.name(s).to_string(), s))
        .filter(|(n, _)| n.ends_with(" (charmed)"))
        .map(|(n, s)| {
            let base = n.trim_end_matches(" (charmed)");
            let pool = ing.store.names.get(base);
            (
                n.to_string(),
                dealt.get(&s).copied().unwrap_or(0),
                taken.get(&s).copied().unwrap_or(0),
                pool.and_then(|p| dealt.get(&p)).copied().unwrap_or(0),
                pool.and_then(|p| taken.get(&p)).copied().unwrap_or(0),
            )
        })
        .collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.1));
    println!(
        "charm instances: {}   unresolved same-name rows: {unresolved}",
        rows.len()
    );
    println!(
        "{:<36} {:>10} {:>10} | {:>10} {:>10}",
        "instance", "pet dealt", "pet taken", "pool dealt", "pool taken"
    );
    for (n, pd, pt, wd, wt) in rows.iter().take(15) {
        println!("{n:<36} {pd:>10} {pt:>10} | {wd:>10} {wt:>10}");
    }
}
