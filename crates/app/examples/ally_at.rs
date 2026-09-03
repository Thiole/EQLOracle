//! why: what the ally class inference says about one player at one
//!      instant of a real log -- the same two calls the ally table makes
//! input: <log> <player> <naive-utc epoch ms>...
//! run: cargo run -p eqlp-app --release --example ally_at -- <log> Kaeus 1787263200000

use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let bytes = std::fs::read(&a[0]).expect("log readable");
    let lines = framed_lines(&bytes);
    let engine = build_engine().expect("pack builds");
    let mut ing = Ingest::default();
    backfill_lines(&mut ing, &engine, &lines, 8);
    eprintln!("last parsed ts = {}", ing.now_ms());
    if a[1] == "chains" {
        let you = ing.store.names.get("You").map(|s| s.0).expect("You");
        let chains = ing.classes.chains(you);
        println!("units={} chains={}", ing.units.len(), chains.len());
        for c in chains.iter().rev().take(12) {
            println!(
                "chain first={:?} last={:?} closed={:?} confirmed={:?} prior={:?} cands={:?} floors={:?} ding={:?} units={} conflicts={} weights={:?}",
                c.first, c.last, c.closed, c.confirmed, c.prior, c.candidates, c.floors, c.max_ding, c.units, c.conflicts, c.weights
            );
        }
        return;
    }
    if a[1] == "recent" {
        let you = ing.store.names.get("You").map(|s| s.0).expect("You");
        let n = ing.units.len();
        for i in n.saturating_sub(8)..n {
            let cfg = ing.classes.configuration_of_visit(you, Some(i));
            let at = ing
                .units
                .bounds(i)
                .map(|(s, e)| e.unwrap_or(s))
                .unwrap_or(ing.now_ms());
            println!(
                "unit {i}: config={cfg:?} level={:?} ding={:?}",
                eqlp_app::combat::you_level_at(&ing, you, &cfg, at),
                ing.levels.at(at)
            );
        }
        return;
    }
    for at in &a[2..] {
        let at: eqlp_source::Millis = at.parse().expect("ms");
        if a[1] == "You" {
            let you = ing.store.names.get("You").map(|s| s.0).expect("You");
            let cfg = ing.classes.configuration_of_visit(you, ing.unit_at(at));
            println!(
                "You at {at}: visit config={cfg:?} level={:?} ding={:?}",
                eqlp_app::combat::you_level_at(&ing, you, &cfg, at),
                ing.levels.at(at)
            );
            continue;
        }
        let (classes, votes) = ing.ally_classes(&a[1], at);
        println!(
            "{} at {at}: inferred={classes:?} votes={votes} who={:?}",
            a[1],
            ing.ally_who(&a[1], at)
        );
    }
}
