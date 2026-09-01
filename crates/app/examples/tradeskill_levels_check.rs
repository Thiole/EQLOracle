//! why: empirical check of tradeskill levels + recent crafts against a real log
//! run: cargo run -p eqlp-app --release --example tradeskill_levels_check -- <log>
use eqlp_app::craftlog::{recent_crafts, tradeskill_levels};
use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: tradeskill_levels_check <log>");
    let engine = build_engine().expect("pack builds");
    let bytes = std::fs::read(&path).expect("read log");
    let lines: Vec<&[u8]> = framed_lines(&bytes);
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let mut ing = Ingest::default();
    for chunk in lines.chunks(100_000) {
        backfill_lines(&mut ing, &engine, chunk, threads);
    }

    println!("-- tradeskill levels --");
    for l in tradeskill_levels(&ing) {
        println!(
            "{}\tlevel={}\tsecondary={}",
            l.skill,
            l.level.map_or("unknown".to_string(), |v| v.to_string()),
            l.secondary
        );
    }
    println!("-- all raw skill levels ({}) --", ing.skill_levels.len());
    let mut all: Vec<_> = ing.skill_levels.iter().collect();
    all.sort();
    for (skill, (level, _)) in all {
        println!("{skill}\t{level}");
    }
    println!("-- recent crafts --");
    for c in recent_crafts(&ing, 15) {
        println!(
            "{}\tskill={}\ticon={}\tts={}",
            c.item,
            c.tradeskill.as_deref().unwrap_or("-"),
            c.icon.as_deref().unwrap_or("-"),
            c.ts_ms
        );
    }
}
