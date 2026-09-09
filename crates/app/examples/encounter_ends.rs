//! why: how long fights stay open after the last kill / last row on a real log
//! input: <log> [last N encounters]
use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;
use eqlp_store::{EncounterId, EventKind};

fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("log");
    let n: usize = a.next().unwrap_or("30".into()).parse().unwrap();
    let raw = std::fs::read(&path).unwrap();
    let lines = framed_lines(&raw);
    let engine = build_engine().unwrap();
    let mut ing = Ingest::default();
    ing.keep_full_history = std::env::var("FULL").is_ok();
    for chunk in lines.chunks(100_000) {
        backfill_lines(&mut ing, &engine, chunk, 4);
    }
    ing.mark_live();
    ing.tick(0);
    let total = ing.store.encounters.len() as u32;
    if std::env::var("SUMMARY").is_ok() {
        let mut buckets = [0usize; 5];
        let mut n = 0usize;
        for id in 0..total {
            let Some(enc) = ing.store.encounter(EncounterId(id)) else {
                continue;
            };
            let Some(end) = enc.end_ms else { continue };
            let mut last_row = enc.start_ms;
            for i in enc.range() {
                if ing.store.enc[i] == id
                    && matches!(
                        ing.store.kind[i],
                        EventKind::Damage | EventKind::Miss | EventKind::Heal
                    )
                {
                    last_row = last_row.max(ing.store.ts[i]);
                }
            }
            // why: a fight that ends ON a death line is over, not lingering
            let mut last_mark = last_row;
            for i in enc.range() {
                if ing.store.enc[i] != id {
                    continue;
                }
                for sym in [ing.store.actor[i], ing.store.target[i]] {
                    for t in ing.timeline.transitions_of(sym.0) {
                        if t.state == eqlp_session::State::Dead
                            && t.ts >= enc.start_ms
                            && t.ts <= end
                        {
                            last_mark = last_mark.max(t.ts);
                        }
                    }
                }
            }
            let gap = (end - last_mark) as f64 / 1000.0;
            n += 1;
            let b = if gap <= 2.5 {
                0
            } else if gap <= 12.0 {
                1
            } else if gap <= 30.0 {
                2
            } else if gap <= 96.0 {
                3
            } else {
                4
            };
            buckets[b] += 1;
            if b >= 1 && std::env::var("SUMMARY").ok().as_deref() == Some("verbose") {
                let mut syms: Vec<eqlp_store::Sym> = Vec::new();
                for i in enc.range() {
                    if ing.store.enc[i] == id {
                        syms.push(ing.store.actor[i]);
                        syms.push(ing.store.target[i]);
                    }
                }
                syms.sort();
                syms.dedup();
                let mut slain = Vec::new();
                let mut up = Vec::new();
                for sym in syms {
                    let name = ing.store.name(sym).to_string();
                    if !ing.allegiance_at(&name, enc.start_ms).is_enemy() {
                        continue;
                    }
                    let dead = ing.timeline.transitions_of(sym.0).iter().any(|t| {
                        t.state == eqlp_session::State::Dead && t.ts >= enc.start_ms && t.ts <= end
                    });
                    if dead {
                        slain.push(name)
                    } else {
                        up.push(name)
                    }
                }
                let fmt = |ms: i64| {
                    let secs = ms / 1000;
                    format!(
                        "{:02}:{:02}:{:02}",
                        (secs / 3600) % 24,
                        (secs / 60) % 60,
                        secs % 60
                    )
                };
                println!(
                    "  gap={gap:>5.1}s enc {id} {:<26} {}-{} lastrow={} slain={slain:?} up={up:?}",
                    ing.store.name(enc.target),
                    fmt(enc.start_ms),
                    fmt(end),
                    fmt(last_row)
                );
            }
        }
        println!(
            "fights={n} end-after-last-row: <=2.5s={} <=12s={} <=30s={} <=96s={} >96s={}",
            buckets[0], buckets[1], buckets[2], buckets[3], buckets[4]
        );
        return;
    }
    println!(
        "{:>5} {:<28} {:>7} {:>9} {:>9} {:>9}  kills",
        "enc", "target", "dur_s", "end-kill", "end-row", "-"
    );
    for id in (0..total).rev().take(n) {
        let Some(enc) = ing.store.encounter(EncounterId(id)) else {
            continue;
        };
        let (mut last_row, mut last_kill, mut kills) = (enc.start_ms, None, Vec::new());
        for i in enc.range() {
            if ing.store.enc[i] != id {
                continue;
            }
            let t = ing.store.ts[i];
            last_row = last_row.max(t);
            if ing.store.kind[i] == EventKind::Death {
                last_kill = Some(t);
                kills.push(ing.store.name(ing.store.target[i]).to_string());
            }
        }
        let end = enc.end_ms.unwrap_or(-1);
        let mut syms: Vec<eqlp_store::Sym> = Vec::new();
        for i in enc.range() {
            if ing.store.enc[i] == id {
                syms.push(ing.store.actor[i]);
                syms.push(ing.store.target[i]);
            }
        }
        syms.sort();
        syms.dedup();
        for sym in syms {
            for t in ing.timeline.transitions_of(sym.0) {
                if t.state == eqlp_session::State::Dead && t.ts >= enc.start_ms && t.ts <= end {
                    last_kill = Some(last_kill.map_or(t.ts, |k: i64| k.max(t.ts)));
                    kills.push(ing.store.name(sym).to_string());
                }
            }
        }
        if std::env::var("ROWS").ok().as_deref() == Some(&id.to_string()) {
            let from = end - 240_000;
            for i in 0..ing.store.len() {
                let t = ing.store.ts[i];
                if t >= from && t <= end + 5_000 {
                    println!(
                        "    row ts={} kind={:?} enc={} {} -> {} {}",
                        t,
                        ing.store.kind[i],
                        ing.store.enc[i],
                        ing.store.name(ing.store.actor[i]),
                        ing.store.name(ing.store.target[i]),
                        ing.store.ability_name(ing.store.ability[i])
                    );
                }
            }
        }
        let fmt = |ms: i64| {
            let secs = ms / 1000;
            let (h, m, s) = ((secs / 3600) % 24, (secs / 60) % 60, secs % 60);
            format!("{:02}:{:02}:{:02}", h, m, s)
        };
        println!(
            "{:>5} {:<28} {:>7.1} {:>9} {:>9} {:>9}  {:?} start={} end={} lastrow={} slain={}",
            id,
            ing.store.name(enc.target),
            (end - enc.start_ms) as f64 / 1000.0,
            last_kill.map_or("-".to_string(), |k| format!(
                "{:.1}",
                (end - k) as f64 / 1000.0
            )),
            format!("{:.1}", (end - last_row) as f64 / 1000.0),
            "-",
            kills,
            fmt(enc.start_ms),
            fmt(end),
            fmt(last_row),
            enc.slain
        );
    }
}
