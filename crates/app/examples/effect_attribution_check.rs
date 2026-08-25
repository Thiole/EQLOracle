//! why: real-data spot check for Ingest::attribute_effect -- dumps
//! recent_effects (source/skill/text) for "You" at several real
//! timestamps across the log, so a real run can be eyeballed for
//! plausibility (a heal attributed to a real healer nearby, not a
//! stranger 3 zones away) rather than trusting synthetic tests alone.
//! input: path to a real log
//! run: cargo run -p eqlp-app --release --example effect_attribution_check -- <log>

use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: effect_attribution_check <log>");
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

    let you = ing.store.names.get("You").expect("You should be interned");
    let all = ing.effects.all(you.0);
    println!("total real effect pings on You: {}", all.len());

    let resolved_source = all.iter().filter(|p| p.source.is_some()).count();
    let resolved_skill = all.iter().filter(|p| p.skill.is_some()).count();
    println!(
        "source resolved: {resolved_source} ({:.1}%)  skill resolved: {resolved_skill} ({:.1}%)",
        100.0 * resolved_source as f64 / all.len().max(1) as f64,
        100.0 * resolved_skill as f64 / all.len().max(1) as f64,
    );

    println!("\n=== last 40 real pings on You ===");
    for p in all.iter().rev().take(40).rev() {
        let src = p.source.as_deref().unwrap_or("?");
        let skill = p.skill.as_deref().unwrap_or("?");
        println!("  {src:>16} > {skill:>24} > {}", p.text);
    }
}
