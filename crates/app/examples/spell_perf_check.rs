//! why: verify SpellPerf against the real log before trusting the
//!      overlay hint -- Cazic partial-resist nights must read as a dip,
//!      Rend must not.
//! input: path to a real log
//! output: final tracker state per spell, ratio, would-flag verdict
//! run: ulimit -v 4000000; cargo run --release -p eqlp-app --example spell_perf_check -- <log>

use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;

fn main() {
    let path = std::env::args().nth(1).unwrap();
    let raw = std::fs::read(&path).unwrap();
    let lines = framed_lines(&raw);
    let engine = build_engine().unwrap();
    let mut ing = Ingest::default();
    for chunk in lines.chunks(100_000) {
        backfill_lines(&mut ing, &engine, chunk, 8);
    }
    let now = ing.now_ms();
    let cur_zone = ing.zone.index_at(now).unwrap_or(usize::MAX);
    let inv = ing.current_invocation.clone().unwrap_or_default();
    println!("current invocation: {:?}", ing.current_invocation);
    let mut rows: Vec<_> = ing.spell_perf.all().collect();
    rows.sort_by_key(|b| std::cmp::Reverse(b.1.landings));
    println!(
        "{:<28} {:>8} {:>8} {:>6} {:>6} {:>8}",
        "spell", "recent", "norm", "n_rec", "landings", ""
    );
    for (name, st) in rows.iter().take(25) {
        println!(
            "{:<28} {:>8.0} {:>8.0} {:>6} {:>8}",
            name, st.ema_recent, st.ema_norm, st.recent_n, st.landings
        );
    }
    let out = ing.spell_perf.check(now, cur_zone, &inv);
    for r in &out.struggling {
        println!(
            "STRUGGLING {} ratio {:.2} recent {:.0} vs baseline {:.0} ({})",
            r.name,
            r.ratio,
            r.recent_avg,
            r.baseline,
            if r.matched {
                "invocation-matched"
            } else {
                "session norm"
            }
        );
    }
    for r in &out.alternatives {
        println!("holding    {} ~{:.0}", r.name, r.baseline);
    }
}
