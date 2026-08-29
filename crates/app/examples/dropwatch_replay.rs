//! why: end-to-end replay verification of the Drop Watch chain against a
//! real log -- mob in combat -> monsterdata drop lookup -> intersection
//! with the player's real tracked items -> the exact rows the overlay
//! widget would render. Simulates the widget's own poll cadence by
//! calling drop_watch after every chunk of lines.
//! run: ... --example dropwatch_replay -- <log> <tracked-item>...

use eqlp_app::dropwatch::drop_watch;
use eqlp_app::ingest::{self, Ingest};
use eqlp_app::parser::build_engine;
use std::collections::BTreeMap;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .expect("usage: dropwatch_replay <log> <item>...");
    let tracked: Vec<String> = args.collect();
    assert!(!tracked.is_empty(), "give at least one tracked item name");

    let raw = std::fs::read(&path).expect("read log");
    let engine = build_engine().expect("pack builds");
    let lines = ingest::framed_lines(&raw);
    let mut ing = Ingest::default();

    // (mob, item) -> (first_seen_ms, ticks_visible)
    let mut alerts: BTreeMap<(String, String), (i64, u64)> = BTreeMap::new();
    // why: ~200 lines per chunk approximates a few seconds of dense
    // combat -- each chunk boundary is one simulated overlay poll
    for chunk in lines.chunks(200) {
        ingest::backfill_lines(&mut ing, &engine, chunk, 1);
        let now = ing.now_ms();
        for row in drop_watch(&ing) {
            for item in &row.drops {
                if tracked.iter().any(|t| t.eq_ignore_ascii_case(item)) {
                    let e = alerts
                        .entry((row.mob.clone(), item.clone()))
                        .or_insert((now, 0));
                    e.1 += 1;
                }
            }
        }
    }

    println!("tracked: {tracked:?}");
    println!("alert windows (mob -> tracked item, first shown, polls visible):");
    for ((mob, item), (first, ticks)) in &alerts {
        let zone = ing.zone.at(*first).unwrap_or("?");
        println!(
            "  {mob} -> {item}  [zone: {zone}] first at log-ms {first}, visible {ticks} polls"
        );
    }
    if alerts.is_empty() {
        println!("  (none -- no tracked item's dropper was ever engaged, or the chain is broken)");
    }

    // ground truth: was any tracked item actually looted in this log?
    use eqlp_store::EventKind;
    let mut looted: BTreeMap<String, u64> = BTreeMap::new();
    for i in 0..ing.store.len() {
        if ing.store.kind[i] == EventKind::Loot {
            let name = ing.store.ability_name(ing.store.ability[i]);
            if tracked.iter().any(|t| t.eq_ignore_ascii_case(name)) {
                *looted.entry(name.to_string()).or_insert(0) += ing.store.amount[i];
            }
        }
    }
    println!("ground truth -- tracked items actually looted in this log: {looted:?}");
}
