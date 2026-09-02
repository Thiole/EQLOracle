//! why: the LIVE path (route + tick per line, as tail_worker does), not
//!      backfill -- prints the meter's encounter at every death line so a
//!      "meter resets when a target dies" report can be reproduced
//! input: <log slice> <HH:MM from> <HH:MM to>
use eqlp_app::combat;
use eqlp_app::ingest::{framed_lines, Ingest};
use eqlp_app::parser::build_engine;
fn m_you(ing: &Ingest) -> Option<u64> {
    combat::live_meter(ing)
        .and_then(|m| m.outgoing.iter().find(|r| r.name == "You").map(|r| r.total))
}
fn hm(s: &str) -> i64 {
    let p: Vec<i64> = s.split(':').map(|v| v.parse().unwrap()).collect();
    p[0] * 3600 + p[1] * 60
}
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (lo, hi) = (hm(&a[1]), hm(&a[2]));
    let raw = std::fs::read(&a[0]).unwrap();
    let lines = framed_lines(&raw);
    let engine = build_engine().unwrap();
    let mut matcher = engine.matcher();
    let mut ing = Ingest::default();
    ing.mark_live();
    let mut wall: i64 = 0;
    let mut last_id: Option<u32> = None;
    let mut last_you: Option<u64> = None;
    for line in &lines {
        let outcome = matcher.classify(line);
        ing.route(&engine, line, &outcome);
        let now = ing.now_ms();
        wall = wall.max(now);
        ing.tick(wall);
        let s = (now / 1000) % 86400;
        if s < lo || s > hi {
            continue;
        }
        let text = String::from_utf8_lossy(line);
        // why: every change of the meter's encounter, plus every death
        let cur_id = combat::current_encounter(&ing).map(|e| e.id.0);
        let changed = cur_id != last_id;
        last_id = cur_id;
        // why: a DROP is the report itself -- your total fell while the
        // meter still says it is the same encounter
        let you_now = m_you(&ing);
        let drop = !changed && you_now.is_some() && last_you.is_some() && you_now < last_you;
        last_you = you_now;
        if text.contains("slain") || changed || drop {
            if changed {
                print!("CHANGE ");
            }
            if drop {
                print!("DROP ");
            }
            let cur = combat::current_encounter(&ing).map(|e| (e.id.0, e.is_open()));
            let m = combat::live_meter(&ing);
            let you = m
                .as_ref()
                .and_then(|m| m.outgoing.iter().find(|r| r.name == "You").map(|r| r.total));
            println!(
                "{} | enc {:?} | meter {:?} dur {}s you={:?}",
                &text[1..20],
                cur,
                m.as_ref().map(|m| m.target.clone()),
                m.as_ref().map(|m| m.duration_ms / 1000).unwrap_or(0),
                you
            );
        }
    }
}
