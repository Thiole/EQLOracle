//! why: the widget polls every second BETWEEN lines with the projected
//!      clock (tick) -- a per-line trace can't see a close that lands in
//!      a gap. Feeds the live route, ticks 1s at a time through every gap,
//!      and prints the first poll after each kill where the meter reads
//!      ended or empty, with the seconds since that kill.
//! input: <log slice> <HH:MM from> <HH:MM to>
use eqlp_app::combat;
use eqlp_app::ingest::{framed_lines, Ingest};
use eqlp_app::parser::build_engine;
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
    let mut last_kill: Option<(i64, String)> = None;
    let mut reported = false;
    for line in &lines {
        let outcome = matcher.classify(line);
        let text = String::from_utf8_lossy(line).to_string();
        ing.route(&engine, line, &outcome);
        let now = ing.now_ms();
        if wall == 0 {
            wall = now;
        }
        // polls through the gap up to this line, 1s apart
        while wall + 1000 <= now {
            wall += 1000;
            ing.tick(wall);
            check(&ing, wall, &last_kill, &mut reported, lo, hi);
        }
        wall = wall.max(now);
        ing.tick(wall);
        // EQLP_DUMP_WINDOW=HH:MM:SS-HH:MM:SS -- per line: graph live fights
        if let Ok(w) = std::env::var("EQLP_DUMP_WINDOW") {
            let (a, b) = w.split_once('-').unwrap();
            let s = (now / 1000) % 86400;
            let hms = |x: &str| -> i64 {
                let p: Vec<i64> = x.split(':').map(|v| v.parse().unwrap()).collect();
                p[0] * 3600 + p[1] * 60 + p[2]
            };
            if s >= hms(a) && s <= hms(b) {
                let live: Vec<String> = ing
                    .encounters
                    .live_encounters()
                    .map(|l| {
                        format!(
                            "{:?}@{} slain={}",
                            l.id,
                            (l.last_ms / 1000) % 86400,
                            l.slain.len()
                        )
                    })
                    .collect();
                let open: Vec<u32> = ing
                    .store
                    .encounters
                    .iter()
                    .filter(|e| e.is_open())
                    .map(|e| e.id.0)
                    .collect();
                println!(
                    "{} | live {:?} | store open {:?} | {}",
                    &text[12..20],
                    live,
                    open,
                    text[27..].chars().take(60).collect::<String>()
                );
            }
        }
        if text.contains("slain") {
            last_kill = Some((now, text[27..].chars().take(40).collect()));
            reported = false;
            // EQLP_DUMP_AT=HH:MM:SS -- dump store + graph state at that kill
            if let Ok(at) = std::env::var("EQLP_DUMP_AT") {
                if text.contains(&at) {
                    for e in &ing.store.encounters {
                        if e.involves_you && (e.is_open() || now - e.end_ms.unwrap_or(0) < 120_000)
                        {
                            println!(
                                "  store enc {} target={} open={} absorbed={} start={} end={:?} last_row_ts={}",
                                e.id.0,
                                ing.store.name(e.target),
                                e.is_open(),
                                e.absorbed,
                                (e.start_ms / 1000) % 86400,
                                e.end_ms.map(|x| (x / 1000) % 86400),
                                ing.store.ts.get(e.last as usize).map(|t| (t / 1000) % 86400).unwrap_or(0)
                            );
                        }
                    }
                    for l in ing.encounters.live_encounters() {
                        println!(
                            "  graph live {:?} last={} slain={:?} ents={:?}",
                            l.id,
                            (l.last_ms / 1000) % 86400,
                            l.slain,
                            l.entities
                        );
                    }
                }
            }
        }
        check(&ing, wall, &last_kill, &mut reported, lo, hi);
    }
}
fn check(
    ing: &Ingest,
    wall: i64,
    last_kill: &Option<(i64, String)>,
    reported: &mut bool,
    lo: i64,
    hi: i64,
) {
    let Some((kt, kname)) = last_kill else { return };
    if *reported {
        return;
    }
    let s = (wall / 1000) % 86400;
    if s < lo || s > hi {
        return;
    }
    let m = combat::live_meter(ing);
    let (open, rows) = m
        .as_ref()
        .map(|m| (m.open, m.outgoing.len()))
        .unwrap_or((false, 0));
    if !open || rows == 0 {
        let since = (wall - kt) as f64 / 1000.0;
        println!(
            "{:02}:{:02}:{:02} {:>5.1}s after '{}' -> open={} rows={} target={:?}",
            s / 3600,
            (s / 60) % 60,
            s % 60,
            since,
            kname.trim(),
            open,
            rows,
            m.as_ref().map(|m| m.target.clone())
        );
        *reported = true;
    }
}
