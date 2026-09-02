//! why: dump recent encounters for one target name -- live debugging
//! input: <log> <target-name-substring>
use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;
fn main() {
    let path = std::env::args().nth(1).unwrap();
    let needle = std::env::args().nth(2).unwrap().to_lowercase();
    let raw = std::fs::read(&path).unwrap();
    let lines = framed_lines(&raw);
    let engine = build_engine().unwrap();
    let mut ing = Ingest::default();
    for chunk in lines.chunks(100_000) {
        backfill_lines(&mut ing, &engine, chunk, 8);
    }
    // why: a third arg dumps every encounter, not just the newest 400
    let all = std::env::args().nth(3).is_some();
    let take = if all { usize::MAX } else { 400 };
    for e in ing.store.encounters.iter().rev().take(take) {
        let t = ing.store.name(e.target).to_lowercase();
        if !t.contains(&needle) {
            continue;
        }
        let fmt = |ms: i64| {
            let s = ms / 1000;
            format!("{:02}:{:02}:{:02}", (s / 3600) % 24, (s / 60) % 60, s % 60)
        };
        let ents = ing.entities_by_enc.get(&e.id).cloned().unwrap_or_default();
        println!(
            "enc {} target={} start={} end={:?} dur={}s slain={} wiped={} open={} absorbed={} entities({})={}",
            e.id.0,
            ing.store.name(e.target),
            fmt(e.start_ms),
            e.end_ms.map(fmt),
            e.end_ms.map(|x| (x - e.start_ms) / 1000).unwrap_or(0),
            e.slain,
            e.wiped,
            e.is_open(),
            e.absorbed,
            ents.len(),
            ents.join(", ")
        );
    }
}
