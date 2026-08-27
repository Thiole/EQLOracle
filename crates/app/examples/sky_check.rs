//! why: verify skyquests::list_class_unlocks against the real Achievements
//!      dump + real inventory dump + real log (live turn-in signal),
//!      not just synthetic fixtures
//! input: base install folder, real log path
//! output: printed per-class unlock status + per-quest completion
//! run: cargo run -p eqlp-app --example sky_check -- <base_dir> <log_path>

use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;
use eqlp_app::skyquests::list_class_unlocks;
use std::path::Path;

fn main() {
    let mut args = std::env::args().skip(1);
    let base_dir = args.next().expect("usage: sky_check <base_dir> <log_path>");
    let log_path = args.next().expect("usage: sky_check <base_dir> <log_path>");

    let engine = build_engine().expect("pack builds");
    let bytes = std::fs::read(&log_path).expect("read log");
    let lines: Vec<&[u8]> = framed_lines(&bytes);
    let mut ing = Ingest::default();
    backfill_lines(&mut ing, &engine, &lines, lines.len());
    println!("live turn-ins this session: {}\n", ing.turn_ins.len());

    let classes = list_class_unlocks(&ing, Some(Path::new(&base_dir)));

    for c in &classes {
        let status = match c.unlocked {
            Some(true) => "UNLOCKED",
            Some(false) => "locked",
            None => "??? (no achievements dump found)",
        };
        println!("{:<14} {}", c.class, status);
        for r in &c.rewards {
            let done = match r.completed {
                Some(true) => "done",
                Some(false) => "open",
                None => "???",
            };
            println!("  [{done}] {:<28} (from {})", r.name, r.quest);
        }
    }
}
