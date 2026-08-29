//! why: empirical check of an item page's loot history against a real
//! log -- the tier-fold fix (strip_tier both sides) claimed most real
//! loot was invisible to the page; this proves the count directly.
//! run: cargo run -p eqlp-app --release --example loot_history_check -- <log> <item>
use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::monsters;
use eqlp_app::parser::build_engine;

fn main() {
    let mut args = std::env::args().skip(1);
    let (path, item) = (
        args.next().expect("usage: loot_history_check <log> <item>"),
        args.next().expect("usage: loot_history_check <log> <item>"),
    );
    let engine = build_engine().expect("pack builds");
    let bytes = std::fs::read(&path).expect("read log");
    let lines: Vec<&[u8]> = framed_lines(&bytes);
    let mut ing = Ingest::default();
    backfill_lines(&mut ing, &engine, &lines, lines.len());

    let events = monsters::item_loot_history(&ing, &item);
    println!("loot history for {item:?}: {} events", events.len());
    let mut by_mob: std::collections::HashMap<&str, u64> = Default::default();
    for e in &events {
        *by_mob.entry(e.mob.as_str()).or_default() += e.qty;
    }
    let mut rows: Vec<_> = by_mob.into_iter().collect();
    rows.sort_by_key(|&(_, n)| std::cmp::Reverse(n));
    for (mob, n) in rows.iter().take(10) {
        println!("  {n:>5}x from {mob}");
    }
}
