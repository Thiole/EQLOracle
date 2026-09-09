//! why: what the scrub panel would list for one encounter -- entities_by_enc
//!      vs the rows' own actors, on a real log
//! input: <log> <encounter id> <seconds after start>
use eqlp_app::combat;
use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;
use eqlp_store::EncounterId;
use std::collections::BTreeSet;

fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("log");
    let id: u32 = a.next().expect("enc id").parse().unwrap();
    let secs: i64 = a.next().unwrap_or("12".into()).parse().unwrap();
    let raw = std::fs::read(&path).unwrap();
    let lines = framed_lines(&raw);
    let engine = build_engine().unwrap();
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let mut ing = Ingest::default();
    for chunk in lines.chunks(100_000) {
        backfill_lines(&mut ing, &engine, chunk, threads);
    }
    ing.mark_live();
    ing.tick(0);
    let enc = ing.store.encounter(EncounterId(id)).expect("encounter");
    println!(
        "enc {id} target={} start={} end={:?}",
        ing.store.name(enc.target),
        enc.start_ms,
        enc.end_ms
    );
    println!(
        "entities_by_enc: {:?}",
        ing.entities_by_enc.get(&EncounterId(id))
    );
    let mut actors: BTreeSet<String> = BTreeSet::new();
    for i in enc.range() {
        if ing.store.enc[i] == id {
            actors.insert(ing.store.name(ing.store.actor[i]).to_string());
            actors.insert(ing.store.name(ing.store.target[i]).to_string());
        }
    }
    println!("row names: {:?}", actors);
    let st = combat::fight_state_at(&ing, id, enc.start_ms + secs * 1000, None);
    for e in st {
        println!(
            "  {:<32} {:<10} dps={:.1} effects={} abilities={}",
            e.name,
            e.state,
            e.dps,
            e.effects.len(),
            e.window_abilities.len()
        );
    }
}
