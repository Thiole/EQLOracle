//! why: "average hp per mob ... solo vs group" -- what the mob page's HP
//!      rows say for named mobs on a real log
//! input: <log> <mob name>...
//! run: cargo run --release -p eqlp-app --example mob_hp_check -- <log> "a gnoll" "Lord Nagafen"
use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::monsters;
use eqlp_app::parser::build_engine;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("log path");
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
    for name in args {
        let s = monsters::mob_stats(&ing, &name);
        println!("{name}: kills={} pulls={}", s.kills, s.pulls);
        for r in s.hp {
            println!(
                "  party {:>2} ({:<5}) kills={:>4} avg={:>8} median={:>8} range={}..{}",
                r.party_size, r.band, r.kills, r.avg_hp, r.median_hp, r.min_hp, r.max_hp
            );
        }
    }
}
