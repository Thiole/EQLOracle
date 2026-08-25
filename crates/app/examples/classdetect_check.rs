//! why: diagnose a real class-detection report against a real log --
//!      exactly the same call the UI itself makes (combat::
//!      class_configurations(ing, "You")), plus a direct scan of every
//!      real spell/song/stance/invocation/skill "You" used that's
//!      eligible for a given target class, to find which one(s) are
//!      actually feeding that candidacy and how big each pool is
//!      (independent of ClassDetector's own internal state, which
//!      keeps no evidence trail once narrowed).
//! input: path to a real log, optional target class (default Beastlord)
//! run: cargo run -p eqlp-app --release --example classdetect_check -- <log> [class]

use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;
use eqlp_app::{classdata, combat, invocationdata, skilldata, stancedata};
use std::collections::HashMap;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: classdetect_check <log> [class]");
    let target = args.next().unwrap_or_else(|| "Beastlord".to_string());

    let raw = std::fs::read(&path).unwrap_or_else(|e| panic!("couldn't read {path}: {e}"));
    let lines = framed_lines(&raw);
    let engine = build_engine().expect("pack builds");
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);

    let mut ing = Ingest::default();
    for chunk in lines.chunks(100_000) {
        backfill_lines(&mut ing, &engine, chunk, threads);
    }

    let you_sym = ing.store.names.get("You").map(|s| s.0);
    eprintln!("YOU symbol = {you_sym:?}");
    let report = combat::class_configurations(&ing, "You");
    println!("=== class_configurations(\"You\") -- the real UI call ===");
    for c in &report.configurations {
        println!(
            "  {:?}  zone_visits={}  level_range={:?}",
            c.classes, c.zone_visits, c.level_range
        );
    }
    println!("  unresolved_visits: {}", report.unresolved_visits);

    // why: independent scan against the exact same real-line shapes
    // ingest.rs's own Action dispatch feeds classdetect for "You":
    // cast.begin (spell), sing.begin (song), state.stance, state.
    // invocation, skill.up. ability.activated is excluded on purpose --
    // its own doc says it's 100% third-person, the log owner never sees
    // their own activation that way.
    println!("\n=== every real spell/song/stance/invocation/skill \"You\" used that's {target}-eligible ===");
    let text = String::from_utf8_lossy(&raw);
    let mut hits: HashMap<String, (u32, usize)> = HashMap::new();
    let roman = |name: &str| -> String {
        name.rsplit_once(' ').map_or(name.to_string(), |(b, tail)| {
            let is_roman = !tail.is_empty() && tail.chars().all(|c| "IVXLCDM".contains(c));
            if is_roman {
                b.to_string()
            } else {
                name.to_string()
            }
        })
    };
    let note = |hits: &mut HashMap<String, (u32, usize)>, label: String, classes: &[String]| {
        if classes.iter().any(|c| c == &target) {
            let entry = hits.entry(label).or_insert((0, classes.len()));
            entry.0 += 1;
        }
    };

    for line in text.lines() {
        let Some(body) = line.split_once("] ").map(|(_, b)| b) else {
            continue;
        };
        if let Some(rest) = body
            .strip_prefix("You begin casting ")
            .or_else(|| body.strip_prefix("You begins casting "))
        {
            let base = roman(rest.trim_end_matches('.'));
            note(
                &mut hits,
                format!("cast: {base}"),
                classdata::classes_for(&base),
            );
        } else if let Some(rest) = body.strip_prefix("You begin singing ") {
            let base = roman(rest.trim_end_matches('.'));
            note(
                &mut hits,
                format!("song: {base}"),
                classdata::classes_for(&base),
            );
        } else if let Some(rest) = body
            .strip_prefix("You assume a ")
            .or_else(|| body.strip_prefix("You assume an "))
        {
            let name = rest.trim_end_matches(" stance.");
            note(
                &mut hits,
                format!("stance: {name}"),
                stancedata::classes_for(name),
            );
        } else if let Some(rest) = body.strip_prefix("You begin reciting the ") {
            let name = rest.trim_end_matches(" invocation.");
            note(
                &mut hits,
                format!("invocation: {name}"),
                invocationdata::classes_for(name),
            );
        } else if let Some(rest) = body.strip_prefix("You have become better at ") {
            let name = rest.split('!').next().unwrap_or(rest);
            note(
                &mut hits,
                format!("skill: {name}"),
                skilldata::classes_for(name),
            );
        }
    }
    let mut hits: Vec<(String, (u32, usize))> = hits.into_iter().collect();
    hits.sort_by_key(|(_, (count, pool_size))| (*pool_size, std::cmp::Reverse(*count)));
    if hits.is_empty() {
        println!("  (none found)");
    }
    for (name, (count, pool_size)) in hits {
        println!("  pool_size={pool_size:>2}  {count:>6}x {name}");
    }
}
