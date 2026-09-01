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
    let mut rows: Vec<_> = ing.spell_perf.all().collect();
    rows.sort_by_key(|b| std::cmp::Reverse(b.1.landings));
    println!(
        "{:<28} {:>8} {:>8} {:>6} {:>6} {:>8} flag",
        "spell", "recent", "norm", "ratio", "n_rec", "landings"
    );
    for (name, st) in rows.iter().take(25) {
        let ratio = if st.ema_norm > 0.0 {
            st.ema_recent / st.ema_norm
        } else {
            1.0
        };
        let active = now - st.last_ms < 10 * 60 * 1000;
        let flag = active && st.recent_n >= 5 && st.landings >= 25 && ratio < 0.75;
        println!(
            "{:<28} {:>8.0} {:>8.0} {:>6.2} {:>6} {:>8} {}",
            name,
            st.ema_recent,
            st.ema_norm,
            ratio,
            st.recent_n,
            st.landings,
            if flag { "STRUGGLING" } else { "" }
        );
    }
}
