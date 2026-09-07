//! why: "a BUNCH of names from old encounters are showing up in current
//!      encounters" -- live_meter folds every fight of yours that
//!      overlaps the current one, and an unclosed fight is measured as
//!      running until NOW, so it overlaps everything forever.
//! input: <log>
use eqlp_app::combat;
use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;

fn main() {
    let path = std::env::args().nth(1).expect("usage: union_check <log>");
    let raw = std::fs::read(&path).expect("log");
    let engine = build_engine().expect("pack");
    let mut ing = Ingest::default();
    for chunk in framed_lines(&raw).chunks(100_000) {
        backfill_lines(&mut ing, &engine, chunk, 8);
    }
    ing.mark_live();
    let now = ing.now_ms();
    let s = &ing.store;
    let open: Vec<&eqlp_store::Encounter> = s
        .encounters
        .iter()
        .filter(|e| e.involves_you && !e.absorbed && e.end_ms.is_none())
        .collect();
    println!(
        "encounters of yours: {} total, {} with NO end_ms",
        s.encounters
            .iter()
            .filter(|e| e.involves_you && !e.absorbed)
            .count(),
        open.len()
    );
    for e in open.iter().take(10) {
        println!(
            "   id={:<6} {:<28} opened {:>7}s ago, last row {:>7}s ago",
            e.id.0,
            s.name(e.target),
            (now - e.start_ms) / 1000,
            (now - s.ts.get(e.last as usize).copied().unwrap_or(e.start_ms)) / 1000
        );
    }
    // why: live_meter's own union rule, applied to every fight in turn --
    // how many OTHER fights each one folds in, and how far back they
    // reach. That is where names from old encounters come from.
    let mine: Vec<&eqlp_store::Encounter> = s
        .encounters
        .iter()
        .filter(|e| e.involves_you && !e.absorbed)
        .collect();
    let last_of = |e: &eqlp_store::Encounter| -> i64 {
        e.end_ms.unwrap_or_else(|| {
            s.ts.get(e.last as usize)
                .copied()
                .unwrap_or(e.start_ms)
                .max(now)
        })
    };
    let mut worst: Vec<(usize, i64, String)> = Vec::new();
    for primary in &mine {
        let mut picked: Vec<&eqlp_store::Encounter> = vec![primary];
        let (mut lo, mut hi) = (primary.start_ms, last_of(primary));
        loop {
            let mut grew = false;
            for e in &mine {
                if picked.iter().any(|x| x.id == e.id) {
                    continue;
                }
                let (a, b) = (e.start_ms, last_of(e));
                if a <= hi && b >= lo {
                    picked.push(e);
                    lo = lo.min(a);
                    hi = hi.max(b);
                    grew = true;
                }
            }
            if !grew {
                break;
            }
        }
        if picked.len() > 1 {
            worst.push((
                picked.len(),
                (primary.start_ms - lo) / 1000,
                s.name(primary.target).to_string(),
            ));
        }
    }
    worst.sort_by_key(|(n, _, _)| std::cmp::Reverse(*n));
    println!(
        "\nfights that fold in others: {} of {}",
        worst.len(),
        mine.len()
    );
    for (n, back, name) in worst.iter().take(10) {
        println!("   {name:<28} folds {n:>3} fights, reaching {back:>6}s before its own start");
    }

    // why: "a BUNCH of names ... in current encounters" -- count the
    // distinct ally actors each fight actually holds rows for
    let mut big: Vec<(usize, String, i64, Vec<String>)> = Vec::new();
    for e in &mine {
        let mut who: std::collections::BTreeSet<String> = Default::default();
        for i in e.range() {
            if s.enc[i] != e.id.0 || s.kind[i] != eqlp_store::EventKind::Damage {
                continue;
            }
            let an = ing.effective_name(s.name(s.actor[i]));
            let tn = s.name(s.target[i]).to_string();
            if !ing.allegiance_at(&an, s.ts[i]).is_enemy()
                && ing.allegiance_at(&tn, s.ts[i]).is_enemy()
            {
                who.insert(an);
            }
        }
        if who.len() >= 6 {
            big.push((
                who.len(),
                s.name(e.target).to_string(),
                (e.end_ms.unwrap_or(now) - e.start_ms) / 1000,
                who.into_iter().collect(),
            ));
        }
    }
    big.sort_by_key(|(n, _, _, _)| std::cmp::Reverse(*n));
    println!("\nfights with 6+ distinct ally actors: {}", big.len());
    for (n, name, dur, who) in big.iter().take(5) {
        println!(
            "   {name:<24} {n:>3} allies over {dur:>4}s: {:?}",
            &who[..who.len().min(12)]
        );
    }

    // why: a fight named after a PERSON -- encounter 558 was "Mythaneil"
    // while the player was hitting a lizard broodling. A fight list full
    // of player names reads exactly like names leaking in from elsewhere.
    let mut people: Vec<(String, i64)> = Vec::new();
    for e in &mine {
        let t = s.name(e.target).to_string();
        if ing.allegiance_at(&t, e.start_ms).is_enemy() {
            continue;
        }
        people.push((t, (now - e.start_ms) / 1000));
    }
    println!(
        "\nfights of yours ANCHORED ON AN ALLY: {} of {}",
        people.len(),
        mine.len()
    );
    for (n, ago) in people.iter().take(12) {
        println!("   {n:<26} opened {ago:>7}s ago");
    }

    // why: the fight the meter is actually showing -- its real span and
    // everyone with a row in it. A component that spans weeks is how a
    // name from a month ago reaches a fight from tonight.
    if let Some(cur) = combat::current_encounter(&ing) {
        let mut who: std::collections::BTreeMap<String, i64> = Default::default();
        let mut rows = 0u32;
        for i in cur.range() {
            if s.enc[i] != cur.id.0 || s.kind[i] != eqlp_store::EventKind::Damage {
                continue;
            }
            rows += 1;
            let an = ing.effective_name(s.name(s.actor[i]));
            let e = who.entry(an).or_insert(s.ts[i]);
            *e = (*e).min(s.ts[i]);
        }
        println!(
            "\ncurrent encounter id={} target={:?}",
            cur.id.0,
            s.name(cur.target)
        );
        println!(
            "   opened {}s ago, {} damage rows, {} distinct actors",
            (now - cur.start_ms) / 1000,
            rows,
            who.len()
        );
        let mut v: Vec<_> = who.into_iter().collect();
        v.sort_by_key(|(_, t)| *t);
        for (n, first) in v.iter().take(16) {
            println!("      {n:<26} first row {:>8}s ago", (now - first) / 1000);
        }
    }

    // why: the reported name, asked for directly -- every fight holding a
    // row for it, and how far that fight spans. A fight that opens in
    // August and still has rows in September is the leak itself.
    if let Ok(want) = std::env::var("EQLP_NAME") {
        println!("\nfights holding a row for {want:?}:");
        for e in s.encounters.iter().filter(|e| !e.absorbed) {
            let mut hit = false;
            let (mut lo, mut hi) = (i64::MAX, i64::MIN);
            let mut actors: std::collections::BTreeSet<String> = Default::default();
            for i in e.range() {
                if s.enc[i] != e.id.0 || s.kind[i] != eqlp_store::EventKind::Damage {
                    continue;
                }
                let an = ing.effective_name(s.name(s.actor[i]));
                if an.eq_ignore_ascii_case(&want) {
                    hit = true;
                }
                actors.insert(an);
                lo = lo.min(s.ts[i]);
                hi = hi.max(s.ts[i]);
            }
            if hit {
                println!(
                    "   id={:<6} target={:<22} started {:>7}h ago, spans {:>5}s, {} actors, zone={:?}",
                    e.id.0,
                    s.name(e.target),
                    (now - e.start_ms) / 3_600_000,
                    (hi - lo) / 1000,
                    actors.len(),
                    e.zone.map(|z| s.name(z).to_string()).unwrap_or_default()
                );
            }
        }
    }

    // why: is ONE encounter one fight? Count the distinct ENEMY targets
    // inside each. The graph builds encounters as connected components,
    // so if A hits X, B hits X and B hits Y, then A, B, X and Y are one
    // component -- one "encounter" spanning mobs you never touched.
    let mut multi: Vec<(usize, usize, String, i64)> = Vec::new();
    for e in &mine {
        let mut foes: std::collections::BTreeSet<String> = Default::default();
        let mut allies: std::collections::BTreeSet<String> = Default::default();
        for i in e.range() {
            if s.enc[i] != e.id.0 || s.kind[i] != eqlp_store::EventKind::Damage {
                continue;
            }
            let an = ing.effective_name(s.name(s.actor[i]));
            let tn = s.name(s.target[i]).to_string();
            if ing.allegiance_at(&tn, s.ts[i]).is_enemy() {
                foes.insert(tn);
            }
            if !ing.allegiance_at(&an, s.ts[i]).is_enemy() {
                allies.insert(an);
            }
        }
        if foes.len() > 1 {
            multi.push((
                foes.len(),
                allies.len(),
                s.name(e.target).to_string(),
                (e.end_ms.unwrap_or(now) - e.start_ms) / 1000,
            ));
        }
    }
    multi.sort_by_key(|(f, _, _, _)| std::cmp::Reverse(*f));
    println!(
        "\nencounters holding MORE THAN ONE enemy: {} of {}",
        multi.len(),
        mine.len()
    );
    for (f, a, name, dur) in multi.iter().take(8) {
        println!("   {name:<26} {f:>3} distinct enemies, {a:>3} allies, {dur:>5}s");
    }

    // why: effective_name folds a pet to its OWNER, and pet_owner is
    // keyed by NAME and never expires -- a pet mapped to someone in
    // August makes every same-named thing today read as that person.
    if let Ok(want) = std::env::var("EQLP_NAME") {
        let owned: Vec<(&str, &str)> = ing
            .inferred_pets()
            .filter(|(_, o)| o.eq_ignore_ascii_case(&want))
            .collect();
        println!("\npet names mapped to owner {want:?}: {}", owned.len());
        for (pet, owner) in owned.iter().take(10) {
            println!("   {pet:<32} -> {owner}");
        }
    }

    match combat::live_meter(&ing) {
        None => println!("\nlive_meter: none"),
        Some(m) => {
            println!(
                "\nlive_meter target={:?}  {} outgoing rows, engagement clock {}s",
                m.target,
                m.outgoing.len(),
                m.duration_ms / 1000
            );
            for r in m.outgoing.iter().take(14) {
                println!("   {:<26} {:>9}", r.name, r.total);
            }
        }
    }
}
