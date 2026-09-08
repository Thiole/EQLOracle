//! why: "theres no 10 man raid" -- who the mob page counts as allies in
//!      each kill of one mob on a real log, after pet folding
//! input: <log> <mob log name> [your character name]
use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;
use std::collections::BTreeSet;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("log path");
    let mob = args.next().expect("mob name");
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
    ing.character = args.next();
    ing.mark_live();
    ing.tick(0);
    let Some(sym) = ing.store.names.get(&mob) else {
        println!("no such mob");
        return;
    };
    let now = ing.now_ms();
    for (pet, owner) in ing.inferred_pets() {
        if owner.eq_ignore_ascii_case("Scarge") {
            println!("inferred pet: {pet} -> {owner}");
        }
    }
    for e in &ing.store.encounters {
        if e.absorbed || e.is_open() {
            continue;
        }
        let present = ing
            .entities_by_enc
            .get(&e.id)
            .is_some_and(|names| names.iter().any(|n| n.eq_ignore_ascii_case(&mob)));
        if !present {
            continue;
        }
        let end = e.end_ms.unwrap_or(now);
        let allies: BTreeSet<String> = eqlp_app::monsters::kill_bodies(&ing, e, sym, end)
            .into_iter()
            .collect();
        let raw: Vec<String> = eqlp_store::by_actor(
            &ing.store,
            &eqlp_store::Filter::encounter(e.id).damage().target(sym),
        )
        .into_iter()
        .map(|(actor, _, _, _)| ing.store.name(actor).to_string())
        .collect();
        if allies.len() > 8 {
            let roster = |ts| -> Vec<String> {
                ing.groups
                    .current_members(ts)
                    .into_iter()
                    .map(|(n, _, _, _)| n)
                    .collect()
            };
            println!(
                "  roster@start={:?} roster@end={:?}",
                roster(e.start_ms),
                roster(e.end_ms.unwrap_or(now))
            );
            println!(
                "enc {} start_ms={} folded={} raw={}\n  folded: {:?}\n  raw:    {:?}",
                e.id.0,
                e.start_ms,
                allies.len(),
                raw.len(),
                allies,
                raw
            );
        }
    }
}
