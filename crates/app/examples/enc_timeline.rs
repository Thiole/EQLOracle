//! why: dump every encounter after a wall-clock HH:MM with anchor,
//!      entities, kill flags and the gap to the next fight you were
//!      in -- the "current fight is just ending mid-fight" report shape
//! input: <log> <HH:MM>
//! run: cargo run --release -p eqlp-app --example enc_timeline -- <log> 22:41
use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;
fn main() {
    let path = std::env::args().nth(1).unwrap();
    let hm = std::env::args().nth(2).unwrap();
    let (h, m) = hm.split_once(':').unwrap();
    let cut_s: i64 = h.parse::<i64>().unwrap() * 3600 + m.parse::<i64>().unwrap() * 60;
    let raw = std::fs::read(&path).unwrap();
    let lines = framed_lines(&raw);
    let engine = build_engine().unwrap();
    let mut ing = Ingest::default();
    for chunk in lines.chunks(100_000) {
        backfill_lines(&mut ing, &engine, chunk, 8);
    }
    let fmt = |ms: i64| {
        let s = ms / 1000;
        format!("{:02}:{:02}:{:02}", (s / 3600) % 24, (s / 60) % 60, s % 60)
    };
    let mut prev_end: Option<i64> = None;
    for e in ing.store.encounters.iter() {
        if (e.start_ms / 1000) % 86400 < cut_s || e.absorbed {
            continue;
        }
        let ents = ing.entities_by_enc.get(&e.id).cloned().unwrap_or_default();
        let gap = prev_end
            .map(|p| format!("{:+.1}s", (e.start_ms - p) as f64 / 1000.0))
            .unwrap_or_default();
        println!(
            "enc {:<4} {}..{} {:>5.0}s you={} slain={} wiped={} gap={} anchor={} | {}",
            e.id.0,
            fmt(e.start_ms),
            e.end_ms.map(fmt).unwrap_or("open".into()),
            e.end_ms
                .map(|x| (x - e.start_ms) as f64 / 1000.0)
                .unwrap_or(0.0),
            e.involves_you as u8,
            e.slain as u8,
            e.wiped as u8,
            gap,
            ing.store.name(e.target),
            ents.join(", ")
        );
        if e.involves_you {
            prev_end = e.end_ms;
        }
    }
}
