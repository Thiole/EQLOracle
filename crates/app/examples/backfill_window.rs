//! why: startup folds the whole log before the app is usable -- this asks
//! how much of that history actually changes what the user sees, by
//! folding only the last N MiB and comparing the answers to the full fold.
//! input: path to a real log
//! run: cargo run -p eqlp-app --release --example backfill_window -- <log>
use eqlp_app::combat;
use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;
use std::time::Instant;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: backfill_window <log>");
    let raw = std::fs::read(&path).unwrap_or_else(|e| panic!("couldn't read {path}: {e}"));
    let engine = build_engine().expect("pack builds");
    let base = std::path::Path::new(&path)
        .parent()
        .and_then(|p| p.parent());

    let mb = 1024 * 1024;
    for want in [raw.len(), 200 * mb, 100 * mb, 50 * mb, 20 * mb, 5 * mb] {
        if want > raw.len() {
            continue;
        }
        // why: byte-sliced tails must start on a line boundary
        let mut cut = raw.len() - want;
        while cut > 0 && raw[cut - 1] != b'\n' {
            cut += 1;
        }
        let slice = &raw[cut..];
        let lines = framed_lines(slice);
        let mut ing = Ingest::default();
        if let Some(b) = base {
            ing.set_spell_file(b);
        }
        let t = Instant::now(); // clock-exempt: benchmark
        for chunk in lines.chunks(100_000) {
            backfill_lines(&mut ing, &engine, chunk, 24);
        }
        ing.mark_live();
        let el = t.elapsed();
        let ts = ing.now_ms();

        let cfg = combat::class_configurations(&ing, "You");
        println!(
            "{:>4} MiB  fold={:>6.2?}  lines={:<8}  zone={:<28} classes={:?} encounters={} kills={} skills={}",
            slice.len() / mb,
            el,
            lines.len(),
            format!("{:?}", ing.zone.at(ts)),
            cfg.configurations.first().map(|c| c.classes.clone()),
            ing.store.encounters.len(),
            ing.store.encounters.iter().filter(|e| e.slain).count(),
            ing.skills.len(),
        );
    }
}
