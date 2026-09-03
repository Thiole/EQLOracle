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
    if a[1] == "recent" {
        let you = ing.store.names.get("You").map(|s| s.0).expect("You");
        let cur = ing.zone.index_at(ing.now_ms()).unwrap_or(0);
        for i in cur.saturating_sub(8)..=cur {
            let cfg = ing.classes.configuration_of_visit(you, Some(i));
            let at = ing.zone.bounds(i).map(|(_, end)| end - 1).unwrap_or(ing.now_ms());
            println!(
                "visit {i}: config={cfg:?} level={:?} ding={:?}",
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
            let cfg = ing
                .classes
                .configuration_of_visit(you, ing.zone.index_at(at));
            println!("You at {at}: visit config={cfg:?}");
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
