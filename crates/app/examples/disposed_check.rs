//! why: verify Ingest::disposed_items (Sky Quests' reward-ownership
//!      inference) against the real reference log -- every vendor-sell
//!      and destroy line still parses, and no real Sky Quest reward
//!      currently reads as disposed by mistake.
//! input: path to a real combat log
//! output: disposed_items count, currency-row count (regression check
//!         for money.vendor_sell's widened pattern), any Sky Quest
//!         reward name that overlaps disposed_items
//! run: cargo run -p eqlp-app --release --example disposed_check -- <log>

use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;
use eqlp_app::skyquests::list_class_unlocks;
use eqlp_store::EventKind;
use std::path::Path;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: disposed_check <log>");
    let engine = build_engine().expect("pack builds");
    let bytes = std::fs::read(&path).expect("read log");
    let lines: Vec<&[u8]> = framed_lines(&bytes);
    let mut ing = Ingest::default();
    backfill_lines(&mut ing, &engine, &lines, lines.len());

    let currency_rows = (0..ing.store.len())
        .filter(|&i| ing.store.kind[i] == EventKind::Currency)
        .count();
    let vendor_rows = (0..ing.store.len())
        .filter(|&i| {
            ing.store.kind[i] == EventKind::Currency
                && ing.store.ability_name(ing.store.ability[i]) == "vendor"
        })
        .count();
    println!("currency rows: {currency_rows} (vendor: {vendor_rows})");
    println!("disposed_items: {}", ing.disposed_items.len());
    for item in &ing.disposed_items {
        println!("  disposed: {item}");
    }

    println!("\n-- Sky Quest rewards whose name overlaps disposed_items --");
    let unlocks = list_class_unlocks(&ing, Some(Path::new("/nonexistent")));
    let mut hit = false;
    for c in &unlocks {
        for r in &c.rewards {
            let key = r.name.to_ascii_lowercase();
            if ing.disposed_items.contains(&key) {
                hit = true;
                println!(
                    "  {} / {}: completed={:?} currently_owned={:?}",
                    c.class, r.name, r.completed, r.currently_owned
                );
            }
        }
    }
    if !hit {
        println!("  (none)");
    }
}
