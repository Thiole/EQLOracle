//! why: prove the STRUGGLING flag fires during the real Cazic
//!      partial-resist stretch and stays quiet for Rend -- end-of-log
//!      state alone can't show that.
//! input: path to a real log
//! output: flag on/off transitions per watched spell, log wall-clock
//! run: ulimit -v 4000000; cargo run --release -p eqlp-app --example spell_perf_timeline -- <log>

use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;

const WATCH: [&str; 4] = ["Conflagration", "Ice Comet", "Rend", "Frost Storm"];

fn fmt(ms: i64) -> String {
    let s = ms / 1000;
    let (d, r) = (s / 86400, s % 86400);
    format!(
        "day{} {:02}:{:02}:{:02}",
        d,
        r / 3600,
        (r % 3600) / 60,
        r % 60
    )
}

fn main() {
    let path = std::env::args().nth(1).unwrap();
    let raw = std::fs::read(&path).unwrap();
    let lines = framed_lines(&raw);
    let engine = build_engine().unwrap();
    let mut ing = Ingest::default();
    let mut flagged = [false; 4];
    for chunk in lines.chunks(2_000) {
        backfill_lines(&mut ing, &engine, chunk, 4);
        let now = ing.now_ms();
        for (i, w) in WATCH.iter().enumerate() {
            let Some((_, st)) = ing.spell_perf.all().find(|(n, _)| n == w) else {
                continue;
            };
            let ratio = if st.ema_norm > 0.0 {
                st.ema_recent / st.ema_norm
            } else {
                1.0
            };
            let on = now - st.last_ms < 10 * 60 * 1000
                && st.recent_n >= 5
                && st.landings >= 25
                && ratio < 0.75;
            if on != flagged[i] {
                flagged[i] = on;
                println!(
                    "{} {w}: {} (ratio {:.2}, recent {:.0} vs norm {:.0}, n_rec {})",
                    fmt(now),
                    if on { "FLAG ON " } else { "flag off" },
                    ratio,
                    st.ema_recent,
                    st.ema_norm,
                    st.recent_n
                );
            }
        }
    }
}
