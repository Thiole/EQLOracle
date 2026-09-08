//! why: "dont see spells/abilities for allies" -- what the ally drill-down
//!      actually returns per ally on a real log, latest visit
//! input: path to a real log
//! run: cargo run --release -p eqlp-app --example ally_abilities_check -- <log>
use eqlp_app::combat;
use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;

fn main() {
    let path = std::env::args().nth(1).expect("log path");
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
    ing.mark_live();
    ing.tick(0);
    let visits = combat::list_zone_visits(&ing);
    // why: the most recent visit with a real group -- solo visits prove nothing
    let mut shown = 0;
    for v in visits.iter().filter_map(|v| v.index) {
        let zv = Some(v as i64);
        let allies = combat::list_allies(&ing, zv, None, false, None);
        if allies.len() < 3 {
            continue;
        }
        println!("visit {v}: {} allies", allies.len());
        for a in allies.iter().take(6) {
            let s = combat::summarize(&ing, zv, None, Some(&a.name), false, None);
            println!(
                "  {:<16} total={:>8} abilities={:>2} casts={:>2}  first={:?}",
                a.name,
                a.total,
                s.abilities.len(),
                s.casts.len(),
                s.abilities.first().map(|r| (
                    r.ability.clone(),
                    r.total,
                    (r.dps * 10.0).round() / 10.0
                ))
            );
        }
        shown += 1;
        if shown == 2 {
            break;
        }
    }
}
