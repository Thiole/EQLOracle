//! why: hard numbers for the audit -- replay the real log once, then time
//! every read path the UI calls, so a claim about cost is measured, not guessed.
//! run: cargo run -p eqlp-app --release --example audit_profile -- <log>

use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;
use std::time::Instant;

fn rss_mb() -> f64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines().find(|l| l.starts_with("VmRSS:")).and_then(|l| {
                l.split_whitespace()
                    .nth(1)
                    .and_then(|v| v.parse::<f64>().ok())
            })
        })
        .map(|kb| kb / 1024.0)
        .unwrap_or(0.0)
}

fn bench<T>(label: &str, iters: u32, mut f: impl FnMut() -> T) -> T {
    // why: one warm call first -- the first touch of a lazy static is not the steady cost
    let mut out = f();
    let t = Instant::now();
    for _ in 0..iters {
        out = f();
    }
    let per = t.elapsed().as_secs_f64() * 1000.0 / f64::from(iters);
    println!("{per:9.2} ms  x{iters:<4} {label}");
    out
}

fn main() {
    let path = std::env::args().nth(1).expect("usage: audit_profile <log>");
    let raw = std::fs::read(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    println!("log {:.1} MB", raw.len() as f64 / 1_048_576.0);

    let t = Instant::now();
    let lines = framed_lines(&raw);
    println!(
        "frame     {:7.2} s  {} lines",
        t.elapsed().as_secs_f64(),
        lines.len()
    );

    let engine = build_engine().expect("pack builds");
    let mut ing = Ingest::default();
    let t = Instant::now();
    for chunk in lines.chunks(100_000) {
        backfill_lines(&mut ing, &engine, chunk, 8);
    }
    let backfill_s = t.elapsed().as_secs_f64();
    ing.mark_live();
    ing.tick(0);
    let line_count = lines.len();
    drop(lines);
    drop(raw);

    let s = &ing.store;
    println!(
        "backfill  {backfill_s:7.2} s  ({:.0} k lines/s)",
        line_count as f64 / backfill_s / 1000.0
    );
    println!("rss       {:7.1} MB", rss_mb());
    println!(
        "store     events={} encounters={} names={} abilities={}",
        s.len(),
        s.encounters.len(),
        s.names.len(),
        s.abilities.len()
    );
    println!("effects   entities={}", ing.effects.entity_count());
    let now = ing.now_ms();
    let visits = eqlp_app::combat::list_zone_visits(&ing);
    let fights = eqlp_app::combat::list_encounters(&ing, None, 0, 50);
    println!(
        "visits={} fights(first page)={}",
        visits.len(),
        fights.len()
    );
    let one = fights.first().map(|f| f.id);
    println!("\n-- read paths, steady-state cost per call --");

    bench("combat::list_zone_visits (whole history)", 5, || {
        eqlp_app::combat::list_zone_visits(&ing)
    });
    bench("combat::list_encounters(None, 0, 50)", 5, || {
        eqlp_app::combat::list_encounters(&ing, None, 0, 50)
    });
    bench("combat::list_allies(all fights)", 3, || {
        eqlp_app::combat::list_allies(&ing, None, None, false, None)
    });
    bench("combat::list_enemies(all fights)", 3, || {
        eqlp_app::combat::list_enemies(&ing, None, None, false, None)
    });
    if let Some(id) = one {
        bench("combat::list_allies(one fight)", 20, || {
            eqlp_app::combat::list_allies(&ing, None, Some(id), false, None)
        });
    }
    bench("combat::live_meter", 20, || {
        eqlp_app::combat::live_meter(&ing)
    });
    bench("raiding::list_raid_rows (per tick)", 3, || {
        eqlp_app::raiding::list_raid_rows(&ing)
    });
    bench("combat::summarize(all fights)", 3, || {
        eqlp_app::combat::summarize(&ing, None, None, None, false, None)
    });
    if let Some(id) = one {
        bench("combat::summarize(one fight)", 20, || {
            eqlp_app::combat::summarize(&ing, None, Some(id), None, false, None)
        });
        bench("combat::fight_timeline(one fight)", 20, || {
            eqlp_app::combat::fight_timeline(&ing, id)
        });
    }
    bench("groupbuffs::group_buffs", 10, || {
        eqlp_app::groupbuffs::group_buffs(&ing, &[], None)
    });
    bench("skilltracker::skill_status", 20, || {
        eqlp_app::skilltracker::skill_status(&ing)
    });
    bench("effects::status_effects", 50, || {
        eqlp_app::effects::status_effects(&ing)
    });
    bench("targeteffects::target_effects", 20, || {
        eqlp_app::targeteffects::target_effects(&ing)
    });
    bench("debugview::game_state", 10, || {
        eqlp_app::debugview::game_state(&ing)
    });
    bench("progression::spellbook", 10, || {
        eqlp_app::progression::spellbook(&ing, &[])
    });
    bench("craftlog::craft_log", 10, || {
        eqlp_app::craftlog::craft_log(&ing)
    });
    bench("dropwatch::drop_watch", 10, || {
        eqlp_app::dropwatch::drop_watch(&ing)
    });
    bench("overview::session", 10, || {
        eqlp_app::overview::session(&ing)
    });
    bench("deathrecap::recap(latest)", 5, || {
        eqlp_app::deathrecap::recap(&ing, None)
    });
    let _ = now;

    println!("\n-- IPC payload size per call (serialized JSON) --");
    let kb = |v: Vec<u8>| v.len() as f64 / 1024.0;
    let mut sizes: Vec<(String, f64)> = vec![
        (
            "list_zone_visits".into(),
            kb(serde_json::to_vec(&visits).unwrap()),
        ),
        (
            "list_encounters(50)".into(),
            kb(serde_json::to_vec(&fights).unwrap()),
        ),
        (
            "list_allies(all fights)".into(),
            kb(serde_json::to_vec(&eqlp_app::combat::list_allies(
                &ing, None, None, false, None,
            ))
            .unwrap()),
        ),
        (
            "live_meter".into(),
            kb(serde_json::to_vec(&eqlp_app::combat::live_meter(&ing)).unwrap()),
        ),
        (
            "group_buffs".into(),
            kb(serde_json::to_vec(&eqlp_app::groupbuffs::group_buffs(&ing, &[], None)).unwrap()),
        ),
        (
            "skill_status".into(),
            kb(serde_json::to_vec(&eqlp_app::skilltracker::skill_status(&ing)).unwrap()),
        ),
        (
            "game_state".into(),
            kb(serde_json::to_vec(&eqlp_app::debugview::game_state(&ing)).unwrap()),
        ),
        (
            "spellbook".into(),
            kb(serde_json::to_vec(&eqlp_app::progression::spellbook(&ing, &[])).unwrap()),
        ),
        (
            "craft_log".into(),
            kb(serde_json::to_vec(&eqlp_app::craftlog::craft_log(&ing)).unwrap()),
        ),
        (
            "target_effects".into(),
            kb(serde_json::to_vec(&eqlp_app::targeteffects::target_effects(&ing)).unwrap()),
        ),
    ];
    sizes.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    for (n, k) in sizes {
        println!("{k:9.1} KB  {n}");
    }

    println!("\n-- Debug db search, per table (browse, no pattern, limit 100) --");
    for table in eqlp_app::dbsearch::TABLES {
        let t = Instant::now();
        let r = eqlp_app::dbsearch::search(&ing, table, "", 100);
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        match r {
            Ok(d) => {
                if ms > 1.0 || d.total > 10_000 {
                    println!(
                        "{ms:9.2} ms  {table:<20} total={} scanned={}",
                        d.total, d.scanned
                    );
                }
            }
            Err(e) => println!("{:>9}     {table:<20} ERR {e}", ""),
        }
    }
    println!("\nrss after {:7.1} MB", rss_mb());
}
