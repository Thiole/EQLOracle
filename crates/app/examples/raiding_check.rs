//! why: verify raiding::list_raids against a real replayed log, not just
//!      the unit tests' fresh-Ingest cases
//! input: path to a real log
//! output: printed raid zones with any non-zero kill/tier/loot completion
//! run: cargo run -p eqlp-app --example raiding_check -- <log>

use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;
use eqlp_app::raiding::{list_raid_rows, RaidTargetDto};

fn fmt_tiers(t: &[bool; 5]) -> String {
    t.iter().map(|&x| if x { '#' } else { '.' }).collect()
}

fn print_target(zone: &str, kind: &str, t: &RaidTargetDto) {
    let looted: usize = t.drops.iter().filter(|d| d.looted).count();
    println!(
        "{:<16} {:<10} {:<25} kills={:<4} solo=[{}] group=[{}] loot={}/{}",
        zone,
        kind,
        t.name,
        t.kills,
        fmt_tiers(&t.solo_tiers_cleared),
        fmt_tiers(&t.group_tiers_cleared),
        looted,
        t.drops.len()
    );
}

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: raiding_check <path-to-log>");
    let raw = std::fs::read(&path).unwrap_or_else(|e| panic!("couldn't read {path}: {e}"));
    let lines = framed_lines(&raw);
    let engine = build_engine().expect("pack builds");
    let mut ing = Ingest::default();
    for chunk in lines.chunks(100_000) {
        backfill_lines(&mut ing, &engine, chunk, 8);
    }

    fn fmt(ms: i64) -> String {
        format!("{}:{:02}", ms / 60_000, (ms / 1000) % 60)
    }

    for row in list_raid_rows(&ing) {
        println!("=== {} ===", row.row);
        for raid in &row.raids {
            let solo: Vec<String> = raid
                .times
                .solo
                .iter()
                .map(|t| {
                    t.as_ref()
                        .map(|t| fmt(t.duration_ms))
                        .unwrap_or_else(|| "--".into())
                })
                .collect();
            let group: Vec<String> = raid
                .times
                .group
                .iter()
                .map(|t| {
                    t.as_ref()
                        .map(|t| fmt(t.duration_ms))
                        .unwrap_or_else(|| "--".into())
                })
                .collect();
            println!(
                "  [{} solo=[{}] group=[{}]]",
                raid.zone,
                solo.join(","),
                group.join(",")
            );
            print_target(&raid.zone, "boss", &raid.boss);
            for m in &raid.minibosses {
                print_target(&raid.zone, "miniboss", m);
            }
        }
    }
}
