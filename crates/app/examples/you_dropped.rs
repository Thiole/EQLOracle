//! why: "it looks like the damage isnt hitting the dps chart unless i hit
//!      the current target but teammates can hit anything" -- at the live
//!      edge, compares what the meter credits YOU against every damage
//!      row you dealt inside the engagement's own time span, broken down
//!      by the target you hit, so a target whose fight the union excludes
//!      shows up by name.
//! input: <log> [HH:MM:SS cutoff]
use eqlp_app::combat;
use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;
use eqlp_store::EventKind;
use std::collections::BTreeMap;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let raw = std::fs::read(&a[0]).expect("log");
    let lines = framed_lines(&raw);
    let cut = a.get(1).cloned();
    let engine = build_engine().expect("pack");
    let mut ing = Ingest::default();
    for chunk in lines.chunks(100_000) {
        let stop = cut.as_ref().and_then(|c| {
            chunk
                .iter()
                .position(|l| String::from_utf8_lossy(l).get(12..20) == Some(c.as_str()))
        });
        backfill_lines(&mut ing, &engine, &chunk[..stop.unwrap_or(chunk.len())], 8);
        if stop.is_some() {
            break;
        }
    }
    ing.mark_live();
    let s = &ing.store;
    let Some(m) = combat::live_meter(&ing) else {
        println!("no live meter");
        return;
    };
    let credited = m
        .outgoing
        .iter()
        .find(|r| r.name == "You")
        .map(|r| r.total)
        .unwrap_or(0);
    let you = s.names.get("You").map(|y| y.0);
    // why: the engagement's own span -- the meter's clock, so this asks
    // "of what you did in this fight, how much did it count"
    let (from, to) = (m.start_ms, m.start_ms + m.duration_ms);
    let mut by_target: BTreeMap<String, (u64, bool)> = BTreeMap::new();
    let mut total = 0u64;
    for i in 0..s.kind.len() {
        if s.kind[i] != EventKind::Damage || Some(s.actor[i].0) != you {
            continue;
        }
        if s.ts[i] < from || s.ts[i] > to {
            continue;
        }
        total += s.amount[i];
        // why: two ways a row of yours can fail to reach the meter --
        // its fight was never folded in, or the sides came out equal at
        // that timestamp so live_meter's own classifier skipped it
        let in_union = s
            .encounter(eqlp_store::EncounterId(s.enc[i]))
            .is_some_and(|e| e.involves_you && !e.absorbed);
        let tn = s.name(s.target[i]).to_string();
        let sided = !ing.allegiance_at("You", s.ts[i]).is_enemy()
            && ing.allegiance_at(&tn, s.ts[i]).is_enemy();
        if !sided {
            println!(
                "   UNSIDED  You -> {} for {} (target reads ALLY at this moment)",
                tn, s.amount[i]
            );
        }
        let counted = in_union && sided;
        // why: the encounter each row is filed under -- a gap means the
        // rows sit in a fight the engagement never folded in
        let e = by_target
            .entry(format!("{} [enc {}]", s.name(s.target[i]), s.enc[i]))
            .or_insert((0, counted));
        e.0 += s.amount[i];
        e.1 &= counted;
    }
    println!(
        "engagement: {:?}  {} s  folding {} fights",
        m.target,
        m.duration_ms / 1000,
        m.target.matches('+').count() + 1
    );
    println!("outgoing rows the widget would draw:");
    for r in &m.outgoing {
        println!(
            "   {:<26} {:>9}  {:>5.1}%  {:>7.0} dps",
            r.name, r.total, r.pct, r.dps
        );
    }
    // why: your own character name must fold into "You" -- a row under
    // the raw name would look like a teammate and read as missing damage
    for i in 0..s.kind.len() {
        if s.kind[i] == EventKind::Damage {
            let n = s.name(s.actor[i]);
            if ing.character.as_deref().is_some_and(|c| c == n) {
                println!("   !! a damage row is filed under {n:?}, not \"You\"");
                break;
            }
        }
    }
    println!();
    println!("meter credits You: {credited}");
    println!("your damage in that span: {total}");
    println!("difference: {}", total as i64 - credited as i64);
    println!("\nby target you hit:");
    let mut rows: Vec<_> = by_target.into_iter().collect();
    rows.sort_by_key(|(_, (n, _))| std::cmp::Reverse(*n));
    for (t, (n, counted)) in rows {
        println!(
            "   {:<30} {:>9}  {}",
            t,
            n,
            if counted {
                ""
            } else {
                "<-- FIGHT NOT IN THE UNION"
            }
        );
    }
}
