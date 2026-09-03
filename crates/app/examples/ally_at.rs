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
    for at in &a[2..] {
        let at: eqlp_source::Millis = at.parse().expect("ms");
        let (classes, votes) = ing.ally_classes(&a[1], at);
        println!(
            "{} at {at}: inferred={classes:?} votes={votes} who={:?}",
            a[1],
            ing.ally_who(&a[1], at)
        );
    }
}
