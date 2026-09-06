//! why: "one fight the ui had me showing as 21.6k damage for a long
//!      time, despite being in combat the rest of the fight" -- runs the
//!      LIVE path and prints the meter's own You total on every line that
//!      changes it, so a plateau in the meter while damage keeps landing
//!      shows up as a gap between the two columns.
//! input: <log slice> <HH:MM from> <HH:MM to>
use eqlp_app::combat;
use eqlp_app::ingest::{framed_lines, Ingest};
use eqlp_app::parser::build_engine;

fn hm(s: &str) -> i64 {
    let p: Vec<i64> = s.split(':').map(|v| v.parse().unwrap()).collect();
    p[0] * 3600 + p[1] * 60
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let raw = std::fs::read(&a[0]).expect("log");
    let (from, to) = (hm(&a[1]), hm(&a[2]));
    let engine = build_engine().expect("pack");
    let mut ing = Ingest::default();
    let mut store_total = 0u64;
    let mut last_meter: Option<u64> = None;
    let you_sym = |ing: &Ingest| ing.store.names.get("You").map(|s| s.0);
    for line in framed_lines(&raw) {
        eqlp_app::ingest::backfill_lines(&mut ing, &engine, &[line], 1);
        let txt = String::from_utf8_lossy(line);
        let Some(stamp) = txt.get(12..20) else {
            continue;
        };
        let secs = match stamp.get(0..8).map(hm_safe) {
            Some(Some(s)) => s,
            _ => continue,
        };
        if secs < from || secs > to {
            continue;
        }
        // why: what the STORE holds for you, independent of the meter
        if let Some(you) = you_sym(&ing) {
            store_total = (0..ing.store.len())
                .filter(|&i| {
                    ing.store.kind[i] == eqlp_store::EventKind::Damage
                        && ing.store.actor[i].0 == you
                })
                .map(|i| ing.store.amount[i])
                .sum();
        }
        let m = combat::live_meter(&ing);
        let meter = m
            .as_ref()
            .and_then(|m| m.outgoing.iter().find(|r| r.name == "You").map(|r| r.total));
        if meter != last_meter {
            println!(
                "{}  meter={:<9} store={:<9} anchor={:?}",
                &txt[1..21],
                meter.map(|v| v.to_string()).unwrap_or_else(|| "-".into()),
                store_total,
                m.as_ref().map(|m| m.target.clone())
            );
            last_meter = meter;
        }
    }
}

fn hm_safe(s: &str) -> Option<i64> {
    let p: Vec<i64> = s.split(':').filter_map(|v| v.parse().ok()).collect();
    (p.len() == 3).then(|| p[0] * 3600 + p[1] * 60 + p[2])
}
