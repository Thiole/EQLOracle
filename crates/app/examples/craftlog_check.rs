//! why: empirical check of the new Craft event kind against a real log
//! run: cargo run -p eqlp-app --release --example craftlog_check -- <log>
use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;
use eqlp_store::EventKind;
use std::collections::HashMap;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: craftlog_check <log>");
    let engine = build_engine().expect("pack builds");
    let bytes = std::fs::read(&path).expect("read log");
    let lines: Vec<&[u8]> = framed_lines(&bytes);
    let mut ing = Ingest::default();
    backfill_lines(&mut ing, &engine, &lines, lines.len());

    let mut by_item: HashMap<String, (u64, u64, u64)> = HashMap::new(); // (attempts, successes, capped)
    for i in 0..ing.store.len() {
        if ing.store.kind[i] != EventKind::Craft {
            continue;
        }
        let name = ing.store.ability_name(ing.store.ability[i]).to_string();
        let success = ing.store.flags[i] & eqlp_store::flag::CRAFT_SUCCESS != 0;
        let capped = ing.store.flags[i] & eqlp_store::flag::CRAFT_SKILL_CAPPED != 0;
        let e = by_item.entry(name).or_insert((0, 0, 0));
        e.0 += 1;
        if success {
            e.1 += 1;
        }
        if capped {
            e.2 += 1;
        }
    }
    let total_rows = by_item.values().map(|(a, _, _)| a).sum::<u64>();
    println!(
        "{} distinct items, {} total craft rows",
        by_item.len(),
        total_rows
    );
    let mut rows: Vec<_> = by_item.into_iter().collect();
    rows.sort_by_key(|(_, (a, _, _))| std::cmp::Reverse(*a));
    for (name, (attempts, successes, capped)) in rows.iter().take(10) {
        println!("{name}\tattempts={attempts}\tsuccesses={successes}\tcapped={capped}");
    }
}
