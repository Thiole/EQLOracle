//! why: verify dpscalc::list_damage_spells against the real catalog +
//!      a real replayed log (so ranks are populated), not just synthetic
//!      unit-test cases -- effect-text parsing across ~2000 spells is
//!      exactly the kind of thing that silently mis-parses a shape this
//!      module's own test cases didn't happen to cover.
//! input: path to a real log
//! output: candidate counts, a few named spot-checks, and the top few by
//!         each metric so an obviously-wrong parse (e.g. a spell with
//!         some absurd DPS from a mis-parsed duration) stands out
//! run: cargo run -p eqlp-app --release --example dpscalc_check -- <log>

use eqlp_app::dpscalc::list_damage_spells;
use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: dpscalc_check <path-to-log>");
    let raw = std::fs::read(&path).unwrap_or_else(|e| panic!("couldn't read {path}: {e}"));
    let lines = framed_lines(&raw);
    let engine = build_engine().expect("pack builds");
    let mut ing = Ingest::default();
    for chunk in lines.chunks(100_000) {
        backfill_lines(&mut ing, &engine, chunk, 8);
    }

    let all = list_damage_spells(&ing, false);
    let (dots, nukes): (Vec<_>, Vec<_>) = all.iter().partition(|s| s.is_dot);
    println!(
        "{} damage candidates total: {} nukes, {} DoTs",
        all.len(),
        nukes.len(),
        dots.len()
    );

    for name in [
        "Ice Comet",
        "Garrison's Mighty Mana Shock",
        "Conflagration",
        "Frost Storm",
    ] {
        match all.iter().find(|s| s.name == name) {
            Some(s) => println!(
                "\n{} (rank {}): total_damage={:.1} mana={:.1} cast={:.2}s recast={:.2}s\n  dpm={:.2} dps_reuse={:.2} dps_ignore={:.2}",
                s.name, s.rank, s.total_damage, s.mana, s.casting_time, s.recast_time, s.dpm, s.dps_with_reuse, s.dps_ignoring_reuse
            ),
            None => println!("\n{name}: NOT FOUND as a damage candidate"),
        }
    }

    println!("\n--- top 10 DoTs by dps_ignoring_reuse (upkeep efficiency) ---");
    let mut dots_sorted = dots.clone();
    dots_sorted.sort_by(|a, b| {
        b.dps_ignoring_reuse
            .partial_cmp(&a.dps_ignoring_reuse)
            .unwrap()
    });
    for s in dots_sorted.iter().take(10) {
        println!(
            "  {:<32} rank={:<2} dur={:.0}s total={:.1} dps_reuse={:.2} dps_ignore={:.2}",
            s.name,
            s.rank,
            s.duration_secs.unwrap_or(0.0),
            s.total_damage,
            s.dps_with_reuse,
            s.dps_ignoring_reuse
        );
    }

    println!("\n--- top 10 nukes by dps_with_reuse ---");
    let mut nukes_sorted = nukes.clone();
    nukes_sorted.sort_by(|a, b| b.dps_with_reuse.partial_cmp(&a.dps_with_reuse).unwrap());
    for s in nukes_sorted.iter().take(10) {
        println!(
            "  {:<32} rank={:<2} total={:.1} mana={:.1} dpm={:.2} dps_reuse={:.2} dps_ignore={:.2}",
            s.name, s.rank, s.total_damage, s.mana, s.dpm, s.dps_with_reuse, s.dps_ignoring_reuse
        );
    }

    println!("\n--- any suspicious outliers (dps_ignoring_reuse > 2000) ---");
    for s in &all {
        if s.dps_ignoring_reuse > 2000.0 {
            println!(
                "  {:<32} is_dot={} rank={} total={:.1} cast={:.2}s dur={:?} dps_ignore={:.2}",
                s.name,
                s.is_dot,
                s.rank,
                s.total_damage,
                s.casting_time,
                s.duration_secs,
                s.dps_ignoring_reuse
            );
        }
    }

    // why: mirrors the frontend's exact class-filter + level-cap +
    // spell-line-dedup + rotation-selection logic, in Rust, against the
    // real data -- a second, independent check that the *end-to-end*
    // pipeline (not just the raw numbers) produces something sane for
    // this character's own real classes, not just eyeballing a raw list.
    let my_classes = ["Wizard", "Enchanter", "Magician"];
    fn line_key(name: &str) -> String {
        let parts: Vec<&str> = name.split(' ').collect();
        if let Some(tail) = parts.last() {
            if !tail.is_empty()
                && tail
                    .bytes()
                    .all(|b| matches!(b, b'I' | b'V' | b'X' | b'L' | b'C' | b'D' | b'M'))
            {
                return parts[..parts.len() - 1].join(" ");
            }
        }
        name.to_string()
    }
    let usable_level = |s: &eqlp_app::dpscalc::DamageSpellDto| -> Option<u32> {
        s.classes
            .iter()
            .filter(|c| my_classes.contains(&c.class.as_str()) && c.level.is_none_or(|l| l <= 50))
            .filter_map(|c| c.level)
            .max()
    };
    let mut by_line: std::collections::HashMap<String, &eqlp_app::dpscalc::DamageSpellDto> =
        std::collections::HashMap::new();
    for s in &all {
        if let Some(lvl) = usable_level(s) {
            let key = line_key(&s.name);
            let better = by_line
                .get(&key)
                .is_none_or(|existing| lvl > usable_level(existing).unwrap_or(0));
            if better {
                by_line.insert(key, s);
            }
        }
    }
    let deduped: Vec<&eqlp_app::dpscalc::DamageSpellDto> = by_line.values().copied().collect();
    println!(
        "\n--- Wizard/Enchanter/Magician, level<=50, deduped: {} candidates ---",
        deduped.len()
    );
    let (dots, nukes): (
        Vec<&&eqlp_app::dpscalc::DamageSpellDto>,
        Vec<&&eqlp_app::dpscalc::DamageSpellDto>,
    ) = deduped.iter().partition(|s| s.is_dot);
    let best_nuke = nukes
        .iter()
        .max_by(|a, b| a.dps_with_reuse.partial_cmp(&b.dps_with_reuse).unwrap());
    if let Some(n) = best_nuke {
        println!(
            "best nuke by dps_with_reuse: {} (rank {}, {:.1} dps)",
            n.name, n.rank, n.dps_with_reuse
        );
        // why: mirrors DpsSuggest.svelte's corrected "worth maintaining"
        // test exactly -- a DoT's total lifetime damage divided by its
        // own casting time (the opportunity cost of that cast), compared
        // against the nuke's own real sustained rate, NOT dps_ignoring_
        // reuse (which now excludes a DoT's tick stream on purpose, see
        // dpscalc.rs's own doc).
        let threshold = n.dps_with_reuse;
        let value = |d: &&eqlp_app::dpscalc::DamageSpellDto| d.total_damage / d.casting_time;
        let mut worthwhile: Vec<_> = dots.iter().filter(|d| value(d) > threshold).collect();
        worthwhile.sort_by(|a, b| value(b).partial_cmp(&value(a)).unwrap());
        println!(
            "DoTs worth maintaining (value/cast-second > nuke's {:.1}):",
            threshold
        );
        for d in worthwhile.iter().take(3) {
            println!(
                "  {} (rank {}, value/cast-sec {:.1}, dur {:.0}s)",
                d.name,
                d.rank,
                value(d),
                d.duration_secs.unwrap_or(0.0)
            );
        }
    } else {
        println!("no usable nuke found");
    }
}
