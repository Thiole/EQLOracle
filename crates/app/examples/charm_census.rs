//! why: every charm instance in the store, its owner and whether any
//! damage row ever reached it -- pets with zero rows never surface
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
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let mut ing = Ingest::default();
    for chunk in lines.chunks(100_000) {
        backfill_lines(&mut ing, &engine, chunk, threads);
    }
    ing.tick(ing.now_ms());

    let mut dealt: HashMap<Sym, u64> = HashMap::new();
    let mut rows: HashMap<Sym, u32> = HashMap::new();
    for i in 0..ing.store.len() {
        if ing.store.kind[i] != EventKind::Damage {
            continue;
        }
        *dealt.entry(ing.store.actor[i]).or_default() += ing.store.amount[i];
        *rows.entry(ing.store.actor[i]).or_default() += 1;
    }

    let mut per_owner: HashMap<String, (u32, u32)> = HashMap::new(); // (instances, with_damage)
    let mut silent: Vec<String> = Vec::new();
    let mut orphan = 0u32;
    for i in 0..ing.store.names.len() {
        let s = Sym(i as u32);
        let n = ing.store.names.name(s);
        if !n.contains("(charmed ") {
            continue;
        }
        let owner = ing
            .encounters
            .entities
            .owner_of(n)
            .map(|o| ing.as_you(o))
            .unwrap_or_else(|| "<no owner>".to_string());
        if owner == "<no owner>" {
            orphan += 1;
        }
        let d = dealt.get(&s).copied().unwrap_or(0);
        let e = per_owner.entry(owner).or_default();
        e.0 += 1;
        if d > 0 {
            e.1 += 1;
        } else {
            silent.push(n.to_string());
        }
    }
    let graph_charms = ing
        .encounters
        .entities
        .all()
        .filter(|(_, _, _, c)| c.is_some())
        .count();
    let mut unresolved = 0u64;
    let mut unresolved_dmg = 0u64;
    for i in 0..ing.store.len() {
        if ing.store.kind[i] == EventKind::Damage
            && ing.store.flags[i] & eqlp_store::flag::UNRESOLVED_INSTANCE != 0
        {
            unresolved += 1;
            unresolved_dmg += ing.store.amount[i];
        }
    }
    println!("charm entities in the GRAPH:        {graph_charms}");
    println!(
        "charm instances interned in STORE:  {}",
        ing.store.names.len() - ing.store.names.len() + {
            let mut n = 0;
            for i in 0..ing.store.names.len() {
                if ing.store.names.name(Sym(i as u32)).contains("(charmed ") {
                    n += 1;
                }
            }
            n
        }
    );
    println!("rows flagged UNRESOLVED_INSTANCE:   {unresolved}  ({unresolved_dmg} damage)\n");

    let mut v: Vec<_> = per_owner.into_iter().collect();
    v.sort_by_key(|(_, (i, _))| std::cmp::Reverse(*i));
    println!("{:<22} {:>10} {:>12}", "owner", "instances", "dealt dmg");
    for (o, (inst, with)) in v.iter() {
        println!("{o:<22} {inst:>10} {with:>12}");
    }
    println!("\ncharm instances with NO owner: {orphan}");
    println!("charm instances that never dealt damage: {}", silent.len());
    for n in silent.iter().take(8) {
        println!("   {n}");
    }
}
