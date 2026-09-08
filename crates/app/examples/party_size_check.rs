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
        let mut allies: BTreeSet<String> = BTreeSet::new();
        let mut raw: Vec<String> = Vec::new();
        for (actor, _, _, _) in eqlp_store::by_actor(
            &ing.store,
            &eqlp_store::Filter::encounter(e.id).damage().target(sym),
        ) {
            let who = ing.store.name(actor).to_string();
            let end = e.end_ms.unwrap_or(now);
            raw.push(format!(
                "{who} kind={:?} enemy@start={} enemy@end={} enemy@now={}",
                ing.encounters.entities.kind(&who),
                ing.allegiance_at(&who, e.start_ms).is_enemy(),
                ing.allegiance_at(&who, end).is_enemy(),
                ing.allegiance_at(&who, now).is_enemy()
            ));
            let owner = ing
                .encounters
                .entities
                .owner_of(&who)
                .or_else(|| ing.pet_of(&who))
                .map(str::to_string);
            let body = owner.unwrap_or(who);
            let body = match &ing.character {
                Some(c) if c.eq_ignore_ascii_case(&body) => "You".to_string(),
                _ => body,
            };
            allies.insert(body);
        }
        if allies.len() > 8 {
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
