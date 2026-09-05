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
    // why: the reported shape -- "its not being attributed to me in the
    // current fight in the overlay". Per fight: what the store kept for
    // You against what the meter's own siding would let through.
    println!("last 25 fights, your damage kept vs sided:");
    let mut recent2: Vec<_> = s.encounters.iter().filter(|e| !e.absorbed).collect();
    recent2.sort_by_key(|e| e.start_ms);
    for e in recent2.iter().rev().take(25) {
        let (mut mine, mut mine_sided) = (0u64, 0u64);
        let mut lost_to: BTreeMap<String, u64> = BTreeMap::new();
        for i in e.range() {
            if s.enc[i] != e.id.0 || s.kind[i] != EventKind::Damage {
                continue;
            }
            let ts = s.ts[i];
            let a = ing.effective_name(s.name(s.actor[i]));
            if !a.eq_ignore_ascii_case("You") {
                continue;
            }
            let t = s.name(s.target[i]).to_string();
            mine += s.amount[i];
            let ae = ing.allegiance_at(&a, ts).is_enemy();
            let te = ing.allegiance_at(&t, ts).is_enemy();
            if !ae && te {
                mine_sided += s.amount[i];
            } else {
                *lost_to.entry(t).or_insert(0) += s.amount[i];
            }
        }
        if mine == 0 {
            continue;
        }
        let note = if lost_to.is_empty() {
            String::new()
        } else {
            let mut v: Vec<_> = lost_to.into_iter().collect();
            v.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
            format!("   LOST -> {:?}", &v[..v.len().min(3)])
        };
        // why: the overlay window is a fixed 360x240 -- rows below the
        // fold are not rendered off-screen, they are clipped away. Where
        // "You" ranks decides whether the player can see their own row.
        let mut totals: BTreeMap<String, u64> = BTreeMap::new();
        for i in e.range() {
            if s.enc[i] != e.id.0 || s.kind[i] != EventKind::Damage {
                continue;
            }
            let ts = s.ts[i];
            let an = ing.effective_name(s.name(s.actor[i]));
            let tn = s.name(s.target[i]).to_string();
            if ing.allegiance_at(&an, ts).is_enemy() || !ing.allegiance_at(&tn, ts).is_enemy() {
                continue;
            }
            *totals.entry(an).or_insert(0) += s.amount[i];
        }
        let mut ranked: Vec<_> = totals.into_iter().collect();
        ranked.sort_by_key(|(_, v)| std::cmp::Reverse(*v));
        let rank = ranked
            .iter()
            .position(|(n, _)| n.eq_ignore_ascii_case("You"))
            .map(|p| p + 1)
            .unwrap_or(0);
        println!(
            "  id={:<5} {:<26} yours={:>7} sided={:>7} allies={:<3} your_rank={}{}",
            e.id.0,
            s.name(e.target),
            mine,
            mine_sided,
            ranked.len(),
            rank,
            note
        );
    }
    // why: "why did i not show up as an active member ... despite
    // casting" -- an ally row exists only for someone who dealt DAMAGE,
    // so a fight spent mezzing, charming or getting resisted has no row
    // for you at all. Emitted as TSV for cross-referencing casts.
    if std::env::args().any(|a| a == "--windows") {
        println!("#TSV\tid\tstart_ms\tend_ms\ttarget\tyour_damage\tallies");
        for e in s.encounters.iter().filter(|x| !x.absorbed) {
            let mut mine = 0u64;
            let mut allies: std::collections::HashSet<String> = Default::default();
            for i in e.range() {
                if s.enc[i] != e.id.0 || s.kind[i] != EventKind::Damage {
                    continue;
                }
                let ts = s.ts[i];
                let an = ing.effective_name(s.name(s.actor[i]));
                let tn = s.name(s.target[i]).to_string();
                if ing.allegiance_at(&an, ts).is_enemy() || !ing.allegiance_at(&tn, ts).is_enemy() {
                    continue;
                }
                if an.eq_ignore_ascii_case("You") {
                    mine += s.amount[i];
                }
                allies.insert(an);
            }
            let mut my_casts = 0u32;
            for i in e.range() {
                if s.enc[i] != e.id.0
                    || s.kind[i] != EventKind::Cast
                    || s.flags[i] & eqlp_store::flag::CAST_LANDED == 0
                {
                    continue;
                }
                if ing
                    .effective_name(s.name(s.actor[i]))
                    .eq_ignore_ascii_case("You")
                {
                    my_casts += 1;
                }
            }
            println!(
                "TSV\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                e.id.0,
                e.start_ms,
                e.end_ms.unwrap_or(e.start_ms),
                s.name(e.target),
                mine,
                allies.len(),
                my_casts
            );
        }
    }
    // why: dump one fight's rows with both sides' allegiance -- an empty
    // ally list means every row sided the same way, and this says which
    if let Some(want) = std::env::args()
        .skip_while(|a| a != "--fight")
        .nth(1)
        .and_then(|a| a.parse::<u32>().ok())
    {
        for e in s.encounters.iter().filter(|e| e.id.0 == want) {
            println!("fight {} target={} :", e.id.0, s.name(e.target));
            for i in e.range() {
                if s.enc[i] != e.id.0 || s.kind[i] != EventKind::Damage {
                    continue;
                }
                let ts = s.ts[i];
                let an = ing.effective_name(s.name(s.actor[i]));
                let tn = s.name(s.target[i]).to_string();
                println!(
                    "   {:<28} (enemy={}) -> {:<28} (enemy={})  {}",
                    an,
                    ing.allegiance_at(&an, ts).is_enemy(),
                    tn,
                    ing.allegiance_at(&tn, ts).is_enemy(),
                    s.amount[i]
                );
            }
        }
    }
    // why: a nearby player is not an enemy -- if one is classified as
    // such, every fight they are in sides enemy->enemy and the meter
    // shows an empty list
    if std::env::args().any(|a| a == "--sides") {
        let mut enemy_named: BTreeMap<String, u64> = BTreeMap::new();
        for i in 0..s.kind.len() {
            if s.kind[i] != EventKind::Damage {
                continue;
            }
            let an = ing.effective_name(s.name(s.actor[i]));
            // why: EQ mob names lead with an article; a bare capitalised
            // name is a player or a named mob
            let bare = !an.starts_with("a ")
                && !an.starts_with("A ")
                && !an.starts_with("an ")
                && !an.starts_with("An ")
                && !an.starts_with("The ")
                && !an.contains(" pet")
                && !an.contains('`');
            if bare && ing.allegiance_at(&an, s.ts[i]).is_enemy() {
                *enemy_named.entry(an).or_insert(0) += s.amount[i];
            }
        }
        let mut v: Vec<_> = enemy_named.into_iter().collect();
        v.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        println!("bare-name actors classified ENEMY: {}", v.len());
        for (n, d) in v.iter().take(12) {
            println!("   {n:<24} {d:>10}");
        }
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
