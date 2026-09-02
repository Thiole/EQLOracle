//! why: row-level view of how fights were assigned in a time window --
//!      ts, kind, actor -> target, encounter id
//! input: <log> <HH:MM:SS from> <HH:MM:SS to>
use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;
fn hms(s: &str) -> i64 {
    let p: Vec<i64> = s.split(':').map(|v| v.parse().unwrap()).collect();
    p[0] * 3600 + p[1] * 60 + p[2]
}
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (lo, hi) = (hms(&a[1]), hms(&a[2]));
    let raw = std::fs::read(&a[0]).unwrap();
    let lines = framed_lines(&raw);
    let engine = build_engine().unwrap();
    let mut ing = Ingest::default();
    for chunk in lines.chunks(100_000) {
        backfill_lines(&mut ing, &engine, chunk, 8);
    }
    for i in 0..ing.store.len() {
        let s = (ing.store.ts[i] / 1000) % 86400;
        if s < lo || s > hi {
            continue;
        }
        println!(
            "{:02}:{:02}:{:02} {:<8} {:<26} -> {:<26} enc {}",
            s / 3600,
            (s / 60) % 60,
            s % 60,
            format!("{:?}", ing.store.kind[i]),
            ing.store.name(ing.store.actor[i]),
            ing.store.name(ing.store.target[i]),
            ing.store.enc[i]
        );
    }
}
