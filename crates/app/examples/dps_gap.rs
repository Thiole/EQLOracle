//! why: "a lot of dps things arent showing up" -- reconcile what the log
//!      says against what the store kept, so a gap points at the stage
//!      that dropped it rather than at a guess
//! input: <log>
//! run: cargo run -p eqlp-app --release --example dps_gap -- <log>
use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;
use eqlp_store::EventKind;
use std::collections::BTreeMap;

fn main() {
    let path = std::env::args().nth(1).expect("usage: dps_gap <log>");
    let raw = std::fs::read(&path).expect("log");
    let lines = framed_lines(&raw);
    let engine = build_engine().expect("pack");
    let mut ing = Ingest::default();
    ing.keep_full_history = true;
    for chunk in lines.chunks(100_000) {
        backfill_lines(&mut ing, &engine, chunk, 8);
    }
    ing.mark_live();
    let s = &ing.store;
    let mut total: u64 = 0;
    let mut by_actor: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    let mut orphan: u64 = 0;
    for i in 0..s.kind.len() {
        if s.kind[i] != EventKind::Damage {
            continue;
        }
        let amt = s.amount[i];
        total += amt;
        if s.enc[i] == u32::MAX {
            orphan += amt;
        }
        let name = s.name(s.actor[i]).to_string();
        let e = by_actor.entry(name).or_insert((0, 0));
        e.0 += amt;
        e.1 += 1;
    }
    println!("store damage events total={total} orphan(no encounter)={orphan}");
    println!(
        "encounters={} kills={}",
        s.encounters.len(),
        s.encounters.iter().filter(|e| e.slain).count()
    );
    // why: involves_you is what decides a fight is YOURS -- a fight the
    // store kept but never surfaces is the shape "dps not being recorded"
    // takes downstream of a healthy parse
    let you = s.names.get("You").map(|y| y.0);
    let mut kept = 0u64;
    let mut dropped = 0u64;
    for e in s.encounters.iter() {
        if e.absorbed {
            continue;
        }
        let mut yours = 0u64;
        for i in (e.first as usize)..=(e.last as usize).min(s.kind.len().saturating_sub(1)) {
            if s.kind[i] != EventKind::Damage || s.enc[i] != e.id.0 {
                continue;
            }
            if Some(s.actor[i].0) == you {
                yours += s.amount[i];
            }
        }
        if e.involves_you {
            kept += yours;
        } else {
            dropped += yours;
        }
    }
    println!("your damage in fights marked yours={kept} in fights NOT marked yours={dropped}");
    let mut recent: Vec<_> = s.encounters.iter().filter(|e| !e.absorbed).collect();
    recent.sort_by_key(|e| e.start_ms);
    println!("last 15 encounters:");
    for e in recent.iter().rev().take(15) {
        let mut dmg = 0u64;
        let mut yours = 0u64;
        for i in (e.first as usize)..=(e.last as usize).min(s.kind.len().saturating_sub(1)) {
            if s.kind[i] != EventKind::Damage || s.enc[i] != e.id.0 {
                continue;
            }
            dmg += s.amount[i];
            if Some(s.actor[i].0) == you {
                yours += s.amount[i];
            }
        }
        println!(
            "  id={:<5} {:<28} yours={:>8} total={:>9} slain={} involves_you={}",
            e.id.0,
            s.name(e.target),
            yours,
            dmg,
            e.slain,
            e.involves_you
        );
    }
    // why: the meter counts a damage row only when actor and target land
    // on OPPOSITE sides -- anything the allegiance model cannot side is
    // silently dropped, which is exactly the shape of "dps not showing up"
    let mut sided = 0u64;
    let mut unsided = 0u64;
    let mut unsided_by: BTreeMap<String, u64> = BTreeMap::new();
    for i in 0..s.kind.len() {
        if s.kind[i] != EventKind::Damage {
            continue;
        }
        let ts = s.ts[i];
        let a = ing.effective_name(s.name(s.actor[i]));
        let t = s.name(s.target[i]).to_string();
        let ae = ing.allegiance_at(&a, ts).is_enemy();
        let te = ing.allegiance_at(&t, ts).is_enemy();
        if (!ae && te) || (ae && !te) {
            sided += s.amount[i];
        } else {
            unsided += s.amount[i];
            *unsided_by.entry(format!("{a} -> {t}")).or_insert(0) += s.amount[i];
        }
    }
    println!("meter-sided damage={sided} unsided(dropped by the meter)={unsided}");
    // why: how much of the drop lands on YOU -- the rest is other
    // people's fights, which the meter was never going to show anyway
    let mut yours_dropped = 0u64;
    for i in 0..s.kind.len() {
        if s.kind[i] != EventKind::Damage {
            continue;
        }
        let ts = s.ts[i];
        let a = ing.effective_name(s.name(s.actor[i]));
        let t = s.name(s.target[i]).to_string();
        let ae = ing.allegiance_at(&a, ts).is_enemy();
        let te = ing.allegiance_at(&t, ts).is_enemy();
        if (!ae && te) || (ae && !te) {
            continue;
        }
        if a.eq_ignore_ascii_case("You") || t.eq_ignore_ascii_case("You") {
            yours_dropped += s.amount[i];
        }
    }
    println!("  of that, damage involving You={yours_dropped}");
    // why: a fight that loses most of its damage is a fight that reads
    // empty, which is what "dps not being recorded" looks like
    let mut gutted = 0usize;
    for e in s
        .encounters
        .iter()
        .filter(|e| !e.absorbed && e.involves_you)
    {
        let (mut kept, mut lost) = (0u64, 0u64);
        for i in e.range() {
            if s.enc[i] != e.id.0 || s.kind[i] != EventKind::Damage {
                continue;
            }
            let ts = s.ts[i];
            let a = ing.effective_name(s.name(s.actor[i]));
            let t = s.name(s.target[i]).to_string();
            let ae = ing.allegiance_at(&a, ts).is_enemy();
            let te = ing.allegiance_at(&t, ts).is_enemy();
            if (!ae && te) || (ae && !te) {
                kept += s.amount[i];
            } else {
                lost += s.amount[i];
            }
        }
        if lost > kept && lost > 0 {
            gutted += 1;
            if gutted <= 8 {
                println!(
                    "   gutted fight id={} {} kept={kept} lost={lost}",
                    e.id.0,
                    s.name(e.target)
                );
            }
        }
    }
    println!("  fights of yours where the meter drops MOST of the damage: {gutted}");
    let mut u: Vec<_> = unsided_by.into_iter().collect();
    u.sort_by_key(|(_, v)| std::cmp::Reverse(*v));
    for (k, v) in u.iter().take(12) {
        println!("   dropped {v:>10}  {k}");
    }
    match eqlp_app::combat::live_meter(&ing) {
        None => println!("live_meter: NONE -- nothing would render"),
        Some(m) => {
            println!(
                "live_meter: open={} target={:?} rows={} incoming={}",
                m.open,
                m.target,
                m.outgoing.len(),
                m.incoming.len()
            );
            for r in m.outgoing.iter().take(10) {
                println!("   {:<24} dmg={:>8} dps={:>8.1}", r.name, r.total, r.dps);
            }
        }
    }
    let mut rows: Vec<_> = by_actor.into_iter().collect();
    rows.sort_by_key(|(_, (amt, _))| std::cmp::Reverse(*amt));
    for (name, (amt, n)) in rows.iter().take(20) {
        println!("  {name:28} {amt:>12} over {n:>7} events");
    }
}
