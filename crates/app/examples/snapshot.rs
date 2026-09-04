//! why: what the app shows when the log is replayed to one instant and
//!      frozen there -- the EQLP_REPLAY_UNTIL state, without the window
//! input: <log> "<Www Mmm DD HH:MM:SS YYYY>"
use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let stamp = format!("[{}] ", a[1]);
    let until = eqlp_core::header::by_name("bracket-ctime")
        .and_then(|h| h.parse(stamp.as_bytes()))
        .map(|(ts, _)| ts.secs() * 1000)
        .expect("a log timestamp");
    let raw = std::fs::read(&a[0]).expect("log");
    let lines = framed_lines(&raw);
    let engine = build_engine().expect("pack");
    let mut ing = Ingest::default();
    let h = eqlp_core::header::by_name("bracket-ctime").expect("header");
    for chunk in lines.chunks(100_000) {
        let cut = chunk
            .iter()
            .position(|l| h.parse(l).is_some_and(|(ts, _)| ts.secs() * 1000 > until))
            .unwrap_or(chunk.len());
        backfill_lines(&mut ing, &engine, &chunk[..cut], 8);
        if cut < chunk.len() {
            break;
        }
    }
    let now = ing.now_ms();
    let gb = eqlp_app::groupbuffs::group_buffs(&ing);
    println!("frozen at {}", a[1]);
    println!("you: {:?} level {:?}", gb.my_classes, eqlp_app::combat::you_level_at(&ing, ing.store.names.get("You").map(|s| s.0).unwrap_or(0), &gb.my_classes, now));
    println!("party:");
    for m in &gb.party {
        println!(
            "  {:<14} {:<28} confirmed={} level={:?} buffs={:?}",
            m.name,
            m.classes.join("/"),
            m.confirmed,
            m.level,
            m.buffs
        );
    }
    println!("group buffs: {}", if gb.good { "Good" } else { "missing some" });
    for r in &gb.rows {
        match &r.active {
            Some(s) => println!("  {:<14} on you: {s}", r.label),
            None => println!(
                "  {:<14} MISSING -- {}",
                r.label,
                r.lines
                    .iter()
                    .map(|l| format!("{} ({})", l.line, l.casters.join(", ")))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
    if let Some(m) = eqlp_app::combat::live_meter(&ing) {
        println!("meter: {} v {} on {:?}", m.ally_count, m.enemy_count, m.current_target);
    }
}
