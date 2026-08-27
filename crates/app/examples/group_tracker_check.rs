//! why: empirical check of GroupTracker against a real log -- confirms
//! the fix actually separates the 3 real standing groupmates from the
//! measured noise (charm mobs, recurring raid trash, one-session
//! strangers) rather than just asserting it in a unit test.
//! run: cargo run -p eqlp-app --release --example group_tracker_check -- <log>
use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: group_tracker_check <log>");
    let engine = build_engine().expect("pack builds");
    let bytes = std::fs::read(&path).expect("read log");
    let lines: Vec<&[u8]> = framed_lines(&bytes);
    let mut ing = Ingest::default();
    backfill_lines(&mut ing, &engine, &lines, lines.len());

    // why: last_ms per actor, not a single global "now" -- GROUP_TTL_MS
    // means asking "grouped as of the log's very last line" is
    // meaningless for anyone whose own activity ended earlier than that;
    // asking "grouped as of THEIR OWN last relevant moment" is the real
    // question of whether the roster would ever have lit them up
    let mut last_ms: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    for i in 0..ing.store.len() {
        if ing.store.kind[i] != eqlp_store::EventKind::Damage {
            continue;
        }
        let name = ing.store.name(ing.store.actor[i]).to_string();
        let ts = ing.store.ts[i];
        last_ms
            .entry(name)
            .and_modify(|t| *t = (*t).max(ts))
            .or_insert(ts);
    }

    let mut grouped: Vec<&str> = last_ms
        .iter()
        .filter(|(n, &ts)| ing.groups.currently_grouped(n, ts))
        .map(|(s, _)| s.as_str())
        .collect();
    grouped.sort();

    println!(
        "=== ever currently_grouped, checked at each name's own last hit ({} of {} distinct actors) ===",
        grouped.len(),
        last_ms.len()
    );
    for name in &grouped {
        let (last, sessions, strong) = ing.groups.evidence_for(name).unwrap();
        println!("{name}\tsessions={sessions}\tstrong={strong}\tlast_ms={last}");
    }
}
