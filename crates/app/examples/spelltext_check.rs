//! why: verify the spelltext dictionary's real-world impact -- exact
//!      count of previously-unmatched lines it now recognizes, against
//!      a real log. Bypasses the shape-cap-obscured coverage report --
//!      counts every Unmatched line directly, not just the top N shapes.
//! run: cargo run --release -p eqlp-app --example spelltext_check -- <log>

use eqlp_app::ingest::framed_lines;
use eqlp_app::parser::build_engine;
use eqlp_app::spelltext::match_spell_text;
use eqlp_core::Outcome;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: spelltext_check <log>");
    let raw = std::fs::read(&path).unwrap_or_else(|e| panic!("couldn't read {path}: {e}"));
    let lines = framed_lines(&raw);
    let engine = build_engine().expect("pack builds");
    let mut matcher = engine.matcher();

    let mut total = 0u64;
    let mut unmatched = 0u64;
    let mut dict_hits = 0u64;
    let mut wearsoff_hits = 0u64;

    for line in &lines {
        total += 1;
        if let Outcome::Unmatched { body, .. } = matcher.classify(line) {
            unmatched += 1;
            let text = String::from_utf8_lossy(body.slice(line));
            if let Some(m) = match_spell_text(&text) {
                dict_hits += 1;
                if m.is_wearsoff {
                    wearsoff_hits += 1;
                }
            }
        }
    }

    println!("total lines:            {total}");
    println!(
        "unmatched by rules:      {unmatched} ({:.2}%)",
        100.0 * unmatched as f64 / total as f64
    );
    println!(
        "of those, spelltext hit: {dict_hits} ({:.2}% of unmatched)  -- {wearsoff_hits} wears-off",
        100.0 * dict_hits as f64 / unmatched as f64
    );
    println!(
        "still unmatched after:   {} ({:.2}%)",
        unmatched - dict_hits,
        100.0 * (unmatched - dict_hits) as f64 / total as f64
    );
}
