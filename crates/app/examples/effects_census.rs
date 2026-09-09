//! why: how often the scrub's buff bar has anything to show on a real log
//! input: <log> [last N encounters]
use eqlp_app::combat;
use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;
use eqlp_store::EncounterId;

fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("log");
    let n: usize = a.next().unwrap_or("30".into()).parse().unwrap();
    let raw = std::fs::read(&path).unwrap();
    let lines = framed_lines(&raw);
    let engine = build_engine().unwrap();
    let mut ing = Ingest::default();
    ing.keep_full_history = std::env::var("FULL").is_ok();
    for chunk in lines.chunks(100_000) {
        backfill_lines(&mut ing, &engine, chunk, 4);
    }
    ing.mark_live();
    ing.tick(0);
    let (mut total, mut with_skill, mut landed) = (0usize, 0usize, 0usize);
    for s in 0..ing.store.names.len() as u32 {
        for p in ing.effects.all(s) {
            total += 1;
            if p.skill.is_some() {
                with_skill += 1;
                if p.landed {
                    landed += 1;
                }
            }
        }
    }
    println!(
        "ledger: {total} pings, {with_skill} with skill, {landed} landed, entities={}",
        ing.effects.entity_count()
    );
    let ids: Vec<u32> = (0..ing.store.encounters.len() as u32)
        .filter(|i| ing.store.encounter(EncounterId(*i)).is_some())
        .collect();
    for id in ids.iter().rev().take(n) {
        let enc = ing.store.encounter(EncounterId(*id)).unwrap();
        let mid = enc.start_ms + (enc.end_ms.unwrap_or(enc.start_ms + 20_000) - enc.start_ms) / 2;
        let st = combat::fight_state_at(&ing, *id, mid, None);
        let eff: Vec<String> = st
            .iter()
            .filter(|e| !e.effects.is_empty())
            .map(|e| format!("{}={}", e.name, e.effects.len()))
            .collect();
        if std::env::var("DETAIL").is_ok() {
            for e in &st {
                for f in &e.effects {
                    println!(
                        "    {} <- {} src={:?} left={:?}",
                        e.name,
                        f.spell,
                        f.source,
                        f.remaining_ms.map(|r| r / 1000)
                    );
                }
            }
        }
        println!(
            "enc {id} {:<30} ents={} with_effects={:?}",
            ing.store.name(enc.target),
            st.len(),
            eff
        );
    }
}
