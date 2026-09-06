//! why: the model cannot grade its own homework. Every other probe here
//!      compares the meter against the store, and the store comes from
//!      the same model -- a shared error reads as zero, which is how a
//!      real 1566-of-3735 loss got reported as "difference: 0" twice.
//!
//!      A RESOLVED fight has something the live path never has:
//!      hindsight. Label every participant from the whole finished fight
//!      at once, independently of what the model believed at the time,
//!      then diff. The gap is the error, measured rather than argued.
//!
//! input: <log> [zone-substring]
//! run: cargo run -p eqlp-app --release --example attribution_backtest -- <log>
use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;
use eqlp_store::EventKind;
use std::collections::{BTreeMap, HashMap, HashSet};

/// why: what the finished fight proves, with no reference to the model
#[derive(Default, Clone, Copy, PartialEq, Debug)]
struct Verdict {
    /// why: it damaged your side, or your side damaged it
    hostile: bool,
    /// why: it damaged something hostile, and never traded with your side
    friendly: bool,
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let raw = std::fs::read(&a[0]).expect("log");
    let zone_filter = a.get(1).cloned();
    let engine = build_engine().expect("pack");
    let mut ing = Ingest::default();
    for chunk in framed_lines(&raw).chunks(100_000) {
        backfill_lines(&mut ing, &engine, chunk, 8);
    }
    ing.mark_live();
    let s = &ing.store;

    let (mut fights, mut agree, mut model_says_ally, mut model_says_enemy) =
        (0u32, 0u64, 0u64, 0u64);
    let mut contested: BTreeMap<String, (u64, u64)> = BTreeMap::new();

    for enc in s.encounters.iter() {
        if enc.absorbed || !enc.involves_you || enc.end_ms.is_none() {
            continue;
        }
        if let Some(f) = &zone_filter {
            let z = enc.zone.map(|z| s.name(z).to_string()).unwrap_or_default();
            if !z.to_lowercase().contains(&f.to_lowercase()) {
                continue;
            }
        }
        // why: the seed is the only thing taken on faith
        // why: "You" alone. A groupmate seed would import the roster's
        // own belief, which is part of what is being judged -- nothing
        // the model concluded gets a vote in its own assessment.
        let seed: HashSet<String> = std::iter::once("You".to_string()).collect();
        let mut rows: Vec<(String, String, u64, i64)> = Vec::new();
        for i in enc.range() {
            if s.enc[i] != enc.id.0 || s.kind[i] != EventKind::Damage {
                continue;
            }
            rows.push((
                ing.effective_name(s.name(s.actor[i])),
                s.name(s.target[i]).to_string(),
                s.amount[i],
                s.ts[i],
            ));
        }
        if rows.is_empty() {
            continue;
        }
        fights += 1;
        // pass 1: anything that traded damage with your side is hostile
        let mut v: HashMap<String, Verdict> = HashMap::new();
        for (an, tn, _, _) in &rows {
            let (a_seed, t_seed) = (seed.contains(an), seed.contains(tn));
            if a_seed && !t_seed {
                v.entry(tn.clone()).or_default().hostile = true;
            }
            if t_seed && !a_seed {
                v.entry(an.clone()).or_default().hostile = true;
            }
        }
        // pass 2: anything that hit a proven-hostile name, and never
        // traded with your side, was fighting for you
        let hostile: HashSet<String> = v
            .iter()
            .filter(|(_, x)| x.hostile)
            .map(|(n, _)| n.clone())
            .collect();
        for (an, tn, _, _) in &rows {
            if seed.contains(an) || hostile.contains(an) {
                continue;
            }
            if hostile.contains(tn) {
                v.entry(an.clone()).or_default().friendly = true;
            }
        }
        // diff every row against what the model believed at that instant
        for (an, _, amt, ts) in &rows {
            let Some(truth) = v.get(an) else { continue };
            let model_enemy = ing.allegiance_at(an, *ts).is_enemy();
            if truth.hostile && truth.friendly {
                let e = contested.entry(an.clone()).or_insert((0, 0));
                if model_enemy {
                    e.1 += amt;
                } else {
                    e.0 += amt;
                }
                continue;
            }
            match (truth.hostile, model_enemy) {
                (true, true) | (false, false) => agree += amt,
                // why: hindsight says it fought you, the model called it an ally
                (true, false) => model_says_ally += amt,
                (false, true) => model_says_enemy += amt,
            }
        }
    }

    println!("resolved fights of yours examined: {fights}");
    println!("damage the model sides as hindsight does: {agree}");
    println!("  hindsight ENEMY, model said ally:  {model_says_ally}");
    println!("  hindsight ALLY,  model said enemy: {model_says_enemy}");
    let total = agree + model_says_ally + model_says_enemy;
    if total > 0 {
        println!(
            "  disagreement: {:.3}% of {total}",
            100.0 * (model_says_ally + model_says_enemy) as f64 / total as f64
        );
    }
    println!("\nnames hindsight labels BOTH ways -- the real ambiguity, not a bug:");
    let mut c: Vec<_> = contested.into_iter().collect();
    c.sort_by_key(|(_, (a, b))| std::cmp::Reverse(a + b));
    for (n, (as_ally, as_enemy)) in c.iter().take(12) {
        println!("   {n:<30} model called it ally {as_ally:>9}, enemy {as_enemy:>9}");
    }
    if c.is_empty() {
        println!("   (none)");
    }
}
