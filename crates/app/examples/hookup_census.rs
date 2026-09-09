//! why: how deep parsed data goes on a real log -- effect pings with a
//! spell, a caster, a catalog record; events outside any encounter
//! input: path to a real log
//! run: cargo run -p eqlp-app --release --example hookup_census -- <log>

use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;
use eqlp_store::NO_ENCOUNTER;
use std::collections::BTreeMap;

fn main() {
    let path = std::env::args().nth(1).expect("usage: hookup_census <log>");
    let raw = std::fs::read(&path).unwrap_or_else(|e| panic!("couldn't read {path}: {e}"));
    let lines = framed_lines(&raw);
    let engine = build_engine().expect("pack builds");
    let mut ing = Ingest::default();
    for chunk in lines.chunks(100_000) {
        backfill_lines(&mut ing, &engine, chunk, 8);
    }
    ing.mark_live();
    ing.tick(0);

    let (mut total, mut skill, mut source, mut both, mut catalog, mut wearoff) = (0, 0, 0, 0, 0, 0);
    let mut unresolved: BTreeMap<String, u64> = BTreeMap::new();
    for (_, p) in ing.effects.iter_all() {
        total += 1;
        if p.skill.is_some() {
            skill += 1;
        }
        if p.source.is_some() {
            source += 1;
        }
        if p.skill.is_some() && p.source.is_some() {
            both += 1;
        }
        if p.skill
            .as_deref()
            .and_then(eqlp_app::spelldata::spell_by_name)
            .is_some()
        {
            catalog += 1;
        }
        if !p.landed {
            wearoff += 1;
        }
        if p.skill.is_none() {
            *unresolved.entry(p.text.to_string()).or_insert(0) += 1;
        }
    }
    println!("effect pings      {total}");
    println!("  skill resolved  {skill}");
    println!("  source resolved {source}");
    println!("  both            {both}");
    println!("  in spell catalog {catalog}");
    println!("  wear-off marks  {wearoff}");
    let mut top: Vec<(String, u64)> = unresolved.into_iter().collect();
    top.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    println!("top unresolved texts");
    for (t, n) in top.iter().take(15) {
        println!("{n:>8}  {t}");
    }

    let s = &ing.store;
    let mut by_kind: BTreeMap<String, (u64, u64, u64)> = BTreeMap::new();
    for i in 0..s.len() {
        let e = by_kind
            .entry(format!("{:?}", s.kind[i]).to_ascii_lowercase())
            .or_insert((0, 0, 0));
        e.0 += 1;
        if s.enc[i] == NO_ENCOUNTER {
            e.1 += 1;
        }
        if s.abilities.get(s.ability[i]).is_none() {
            e.2 += 1;
        }
    }
    println!("\nevents by kind: total / outside any encounter / no ability");
    for (k, (t, o, a)) in by_kind {
        println!("{k:<14} {t:>8} {o:>8} {a:>8}");
    }
}
