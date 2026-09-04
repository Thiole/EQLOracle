//! why: verify the Epic Quests tab's data against the real log +
//!      inventory dump before trusting the tab -- owned/looted statuses
//!      must reflect reality, not just parse.
//! input: path to a real log, game base dir (for inventory/achievements)
//! output: per-class farm-list status counts; every owned/looted item named
//! run: ulimit -v 4000000; cargo run --release -p eqlp-app --example epic_check -- <log> <base_dir>

use eqlp_app::epicquests;
use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;
use std::path::Path;

fn main() {
    let log = std::env::args().nth(1).unwrap();
    let base = std::env::args().nth(2);
    let raw = std::fs::read(&log).unwrap();
    let lines = framed_lines(&raw);
    let engine = build_engine().unwrap();
    let mut ing = Ingest::default();
    for chunk in lines.chunks(100_000) {
        backfill_lines(&mut ing, &engine, chunk, 8);
    }
    let epics = epicquests::list_epics(&ing, base.as_deref().map(Path::new));

    // why: out of era is the default now -- this is the whole ledger of
    // what survived the acquisition chain, and why the rest didn't
    let all: Vec<_> = epics.iter().flat_map(|c| c.items.iter()).collect();
    let farmable = all.iter().filter(|i| i.in_era).count();
    println!(
        "\nera: {} materials, {farmable} farmable now, {} out of era",
        all.len(),
        all.len() - farmable
    );
    for c in &epics {
        for i in c.items.iter().filter(|i| !i.in_era) {
            let why = i
                .unverified
                .clone()
                .unwrap_or_else(|| format!("gated behind {}", i.era.as_deref().unwrap_or("?")));
            println!("  {:<13} {:<36} {why}", c.class, i.status.item);
        }
    }
    for c in &epics {
        let owned = c
            .items
            .iter()
            .filter(|i| i.status.currently_owned.unwrap_or(0) > 0)
            .count();
        let looted = c.items.iter().filter(|i| i.status.ever_looted).count();
        println!(
            "{:<14} items={:<2} looted-ever={:<2} owned-now={}",
            c.class,
            c.items.len(),
            looted,
            owned
        );
        for i in &c.items {
            if i.status.ever_looted || i.status.currently_owned.unwrap_or(0) > 0 {
                println!(
                    "    {} looted x{} owned {:?} sold={}",
                    i.status.item,
                    i.status.looted_count,
                    i.status.currently_owned,
                    i.status.sold_without_keeping
                );
            }
        }
    }
}
