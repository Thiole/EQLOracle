//! why: real-data spot check for combat::class_configurations -- dumps
//! every resolved configuration's own level_range plus the unresolved
//! visit count, so a real "why didn't my level update" report can be
//! traced to an actual folding/evidence gap instead of guessed at.
//! input: path to a real log, character name
//! run: cargo run -p eqlp-app --release --example class_level_check -- <log> <name>

use eqlp_app::combat;
use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: class_level_check <log> <name>");
    let name = args.next().unwrap_or_else(|| "You".to_string());
    let raw = std::fs::read(&path).unwrap_or_else(|e| panic!("couldn't read {path}: {e}"));
    let lines = framed_lines(&raw);
    let engine = build_engine().expect("pack builds");
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);

    let mut ing = Ingest::default();
    // why: L8 reads the install's own spells_us.txt -- <install>/Logs/<log>
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

    let dto = combat::class_configurations(&ing, &name);
    println!("unresolved_visits: {}", dto.unresolved_visits);
    for c in &dto.configurations {
        println!(
            "{:>3} visits  {:?}  level_range={:?}",
            c.zone_visits, c.classes, c.level_range
        );
    }

    if let Some(sym) = ing.store.names.get(&name) {
        let chains = ing.classes.chains(sym.0);
        let mut swap = 0;
        let mut contra = 0;
        let mut open = 0;
        for c in &chains {
            match c.closed {
                Some(eqlp_session::classdetect::ChainEnd::Swap) => swap += 1,
                Some(eqlp_session::classdetect::ChainEnd::Contradiction) => contra += 1,
                None => open += 1,
            }
        }
        println!(
            "\nchains: {} (swap-closed {swap}, contradiction-closed {contra}, open {open})",
            chains.len()
        );
    }
    if let Some(sym) = ing.store.names.get(&name) {
        println!("\nrolling record (L1-L4):");
        for (c, l) in ing.classes.class_levels(sym.0) {
            println!("  {c:16} {l}");
        }
    }
    // why: optional 3rd arg -- dump the chain as the row reads it at one
    // instant, weights included, to trace a class the row should not hold
    if let Some(at) = args.next().and_then(|a| a.parse::<i64>().ok()) {
        if let Some(sym) = ing.store.names.get(&name) {
            let unit = ing.unit_at(at);
            println!("\nchain at {at} (unit {unit:?}):");
            if let Some(v) = ing.classes.chain_at(sym.0, unit) {
                println!("  first={:?} last={:?} closed={:?}", v.first, v.last, v.closed);
                println!("  confirmed={:?} prior={:?}", v.confirmed, v.prior);
                println!("  candidates={:?} leading={:?}", v.candidates, v.leading);
                println!("  who={:?} units={} conflicts={}", v.who, v.units, v.conflicts);
                println!("  floors={:?} max_ding={:?}", v.floors, v.max_ding);
                println!("  weights={:?}", v.weights);
            }
        }
    }
    if let Some(sym) = ing.store.names.get(&name) {
        for c in ["Necromancer", "Shadow Knight", "Bard", "Paladin"] {
            let trail = ing.classes.level_trail(sym.0, c);
            println!("\n{c} trail ({} stamps):", trail.len());
            for (u, l, t) in trail {
                let ts = u.and_then(|i| ing.units.bounds(i)).map(|(s, _)| s);
                println!("  unit={u:?} ts={ts:?} level={l} tier={t:?}");
            }
        }
    }
    println!("\nlatest level: {:?}", ing.levels.latest());
    println!("latest level ts: {:?}", ing.levels.latest_ts());

    // why: which zone visit is currently open (the last one), and its
    // own start ts -- for cross-checking against the level.up ts above
    if let Some(i) = ing.zone.len().checked_sub(1) {
        if let Some((start, next)) = ing.zone.bounds(i) {
            println!("last zone visit index {i}: start={start} next={next:?}");
        }
    }

    // why: optional 3rd arg -- which bucket (full/unresolved) a specific
    // visit index landed in, for tracing a real "why is this one visit
    // ambiguous" report down to ground truth
    if let Some(want) = args.next().and_then(|s| s.parse::<usize>().ok()) {
        let sym = ing.store.names.get(&name).expect("name should exist");
        let (resolved, unresolved) = ing.classes.visits_by_resolved_configuration(sym.0);
        println!("\nvisit index {want}:");
        if let Some((start, next)) = ing.zone.bounds(want) {
            println!("  zone bounds: start={start} next={next:?}");
        }
        let mut found = false;
        for (classes, visits) in &resolved {
            if visits.contains(&Some(want)) {
                println!("  resolved -> {classes:?}");
                found = true;
            }
        }
        if unresolved.contains(&Some(want)) {
            println!("  unresolved");
            found = true;
        }
        if !found {
            println!("  not present in any bucket (no class evidence touched this visit at all)");
        }
    }
}
