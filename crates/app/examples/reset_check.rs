//! why: measure how trigger-happy the 10s encounter idle close really is
//!      against a real log, before changing it -- reported live: mezz
//!      lulls and fresh zone-ins read as "reset" too eagerly.
//! input: path to a real log
//! output: outcome counts; premature-reset evidence (same target
//!         re-engaged shortly after a reset close, gap distribution,
//!         what idle_ms would have absorbed them); mezz/zone-in overlap
//! run: cargo run --release -p eqlp-app --example reset_check -- <log>

use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;

/// why: log-line timestamp -> epoch-ish ms, same shape the engine stamps
fn parse_ts(line: &[u8]) -> Option<i64> {
    // [Tue Jul 28 15:02:15 2026]  -- cheap fixed-offset parse, probe-only
    let s = std::str::from_utf8(line.get(1..25)?).ok()?;
    let mon = match &s[4..7] {
        "Jan" => 1,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12,
        _ => return None,
    };
    let day: i64 = s[8..10].trim().parse().ok()?;
    let h: i64 = s[11..13].parse().ok()?;
    let m: i64 = s[14..16].parse().ok()?;
    let sec: i64 = s[17..19].parse().ok()?;
    let year: i64 = s[20..24].parse().ok()?;
    // days since epoch, civil-from-days inverse (probe precision is fine)
    let y = if mon <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (mon + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some(((days * 24 + h) * 60 + m) * 60_000 + sec * 1000)
}

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: reset_check <path-to-log>");
    let raw = std::fs::read(&path).unwrap_or_else(|e| panic!("couldn't read {path}: {e}"));
    let lines = framed_lines(&raw);

    // pass 1: mezz landings, straight off the raw lines (probe-grade)
    let mut mezzes: Vec<(i64, String)> = Vec::new();
    for l in &lines {
        if let Some(pos) = l.windows(20).position(|w| w == b" has been mesmerized") {
            if let (Some(ts), Ok(who)) = (parse_ts(l), std::str::from_utf8(&l[27..pos])) {
                mezzes.push((ts, who.to_lowercase()));
            }
        }
    }

    let engine = build_engine().expect("pack builds");
    let mut ing = Ingest::default();
    for chunk in lines.chunks(100_000) {
        backfill_lines(&mut ing, &engine, chunk, 8);
    }

    let encs: Vec<_> = ing
        .store
        .encounters
        .iter()
        .filter(|e| e.end_ms.is_some())
        .collect();
    let slain = encs.iter().filter(|e| e.slain).count();
    let wiped = encs.iter().filter(|e| e.wiped && !e.slain).count();
    let resets: Vec<_> = encs.iter().filter(|e| !e.slain && !e.wiped).collect();
    println!(
        "closed encounters: {} -- kills {} ({:.1}%), wipes {}, resets {} ({:.1}%)",
        encs.len(),
        slain,
        100.0 * slain as f64 / encs.len() as f64,
        wiped,
        resets.len(),
        100.0 * resets.len() as f64 / encs.len() as f64
    );

    // premature-reset evidence: same target re-engaged within 120s of a
    // reset close -- the fight plainly wasn't over
    let mut gaps: Vec<i64> = Vec::new();
    let mut resumed_kill = 0usize;
    let mut mezz_overlap = 0usize;
    for r in &resets {
        let tname = ing.store.name(r.target).to_lowercase();
        let end = r.end_ms.unwrap();
        if let Some(next) = ing
            .store
            .encounters
            .iter()
            .filter(|n| n.id.0 > r.id.0 && n.target == r.target && n.start_ms >= end)
            .min_by_key(|n| n.start_ms)
        {
            let gap = next.start_ms - end;
            if gap <= 120_000 {
                gaps.push(gap);
                if next.slain {
                    resumed_kill += 1;
                }
            }
        }
        // was this target mezzed in the 20s leading into the close?
        if mezzes
            .iter()
            .any(|(ts, who)| who == &tname && *ts <= end && end - *ts <= 20_000)
        {
            mezz_overlap += 1;
        }
    }
    gaps.sort_unstable();
    println!(
        "resets re-engaged (same target) within 120s: {} ({:.1}% of resets); {} of those chains ended in a KILL",
        gaps.len(),
        100.0 * gaps.len() as f64 / resets.len().max(1) as f64,
        resumed_kill
    );
    println!(
        "resets with the target mezzed within 20s of the close: {} ({:.1}%)",
        mezz_overlap,
        100.0 * mezz_overlap as f64 / resets.len().max(1) as f64
    );
    if !gaps.is_empty() {
        let pct = |p: f64| gaps[((gaps.len() - 1) as f64 * p) as usize];
        println!(
            "re-engage gap after close: p50={}s p75={}s p90={}s max={}s",
            pct(0.5) / 1000,
            pct(0.75) / 1000,
            pct(0.9) / 1000,
            gaps.last().unwrap() / 1000
        );
        for extra in [5_000i64, 10_000, 20_000, 50_000] {
            let absorbed = gaps.iter().filter(|&&g| g <= extra).count();
            println!(
                "  idle_ms {}s (now 10s) would have absorbed {} of {} splits",
                10 + extra / 1000,
                absorbed,
                gaps.len()
            );
        }
    }

    // zone-in adjacency: resets whose fight STARTED within 30s of a zone line
    let zone_starts: Vec<i64> = ing.zone.iter().map(|(ts, _)| ts).collect();
    let near_zone = resets
        .iter()
        .filter(|r| zone_starts.iter().any(|z| (r.start_ms - z).abs() <= 30_000))
        .count();
    println!(
        "resets whose fight began within 30s of a zone-in: {} ({:.1}%)",
        near_zone,
        100.0 * near_zone as f64 / resets.len().max(1) as f64
    );
}
