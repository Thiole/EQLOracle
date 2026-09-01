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
    for e in ing.store.encounters.iter().rev().take(400) {
        let t = ing.store.name(e.target).to_lowercase();
        if !t.contains(&needle) {
            continue;
        }
        let fmt = |ms: i64| {
            let s = ms / 1000;
            format!("{:02}:{:02}:{:02}", (s / 3600) % 24, (s / 60) % 60, s % 60)
        };
        println!(
            "enc {} target={} start={} end={:?} slain={} wiped={} open={} absorbed={}",
            e.id.0,
            ing.store.name(e.target),
            fmt(e.start_ms),
            e.end_ms.map(fmt),
            e.slain,
            e.wiped,
            e.is_open(),
            e.absorbed
        );
    }
}
