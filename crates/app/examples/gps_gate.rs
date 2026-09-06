//! why: "sometimes it suggests out of era locations" and "players who
//!      have wiz/druid leveled aren't being suggested that they can use
//!      those ports" -- says which gate each teleport dies at, and how
//!      many destinations the picker offers that the era rules out.
//! input: <log>
//! run: cargo run -p eqlp-app --release --example gps_gate -- <log>
use eqlp_app::gearplanner::{era_ix, CURRENT_ERA};
use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;
use eqlp_app::{combat, teleportdata, zonedata};
use std::collections::HashSet;

fn main() {
    let path = std::env::args().nth(1).expect("usage: gps_gate <log>");
    let raw = std::fs::read(&path).expect("log");
    let lines = framed_lines(&raw);
    let engine = build_engine().expect("pack");
    let mut ing = Ingest::default();
    for chunk in lines.chunks(100_000) {
        backfill_lines(&mut ing, &engine, chunk, 8);
    }
    ing.mark_live();

    // ---- the destination picker's own list, against the era ceiling
    let live = era_ix(CURRENT_ERA);
    let zones = zonedata::zones();
    let out: Vec<&str> = zones
        .iter()
        .filter(|z| {
            z.era
                .as_deref()
                .and_then(era_ix)
                .is_some_and(|ix| live.is_some_and(|l| ix > l))
        })
        .map(|z| z.name.as_str())
        .collect();
    println!(
        "destination picker offers {} zones; {} are past {CURRENT_ERA}",
        zones.len(),
        out.len()
    );
    for n in out.iter().take(10) {
        println!("   out of era: {n}");
    }

    // ---- exactly what find_zone_route computes
    let dto = combat::class_configurations(&ing, "You");
    let known: HashSet<String> = ing
        .spellbook
        .known()
        .map(|(n, _)| n.to_ascii_lowercase())
        .collect();
    let (classes, level) = dto
        .configurations
        .first()
        .map(|c| {
            (
                c.classes.clone(),
                c.level_range.map(|(_, hi)| hi).unwrap_or(0),
            )
        })
        .unwrap_or_default();
    println!(
        "\nclasses={classes:?} level={level} known_spells={}",
        known.len()
    );

    println!("all configurations class_configurations reports:");
    for c in &dto.configurations {
        println!(
            "   {:?} level_range={:?} zone_visits={}",
            c.classes, c.level_range, c.zone_visits
        );
    }
    let you = ing.store.names.get("You").map(|s| s.0).unwrap_or(0);
    println!(
        "combat::you_level_at (what the Character page shows) = {:?}",
        combat::you_level_at(&ing, you, &classes, ing.now_ms())
    );

    // why: per-class levels -- "a 30 druid and wizard get all the
    // portals ... depends if the player has them unlocked/leveled"
    let per_class: Vec<(String, u8)> = ing.classes.class_levels(you);
    println!("per-class levels: {per_class:?}");

    // why: "some zones aren't in yet" -- a landing whose destination
    // never resolves to a zonedata name can never become a graph edge
    // why: the same three steps routing::resolve_zone_name runs -- exact
    // name, then its alias table, then a map shortname off who_name
    let aliases: &[(&str, &str)] = &[
        ("Cazic Thule", "Cazic Thule (Zone)"),
        ("Temple of Cazic-Thule", "Cazic Thule (Zone)"),
        ("North Karana", "Northern Plains of Karana"),
        ("East Karana", "Eastern Plains of Karana"),
        ("The Southern Plains of Karana", "Southern Karana"),
        ("The Western Plains of Karana", "Western Karana"),
        ("South Ro", "Southern Desert of Ro"),
        ("North Ro", "The Northern Desert of Ro"),
        ("West Karana", "Western Karana"),
        ("The City of Guk", "Upper Guk"),
        ("The Ruins of Old Guk", "Lower Guk"),
        ("The Feerott", "The Feerrott"),
        ("Wakening Lands", "The Wakening Land"),
        ("The Lair of the Splitpaw", "Splitpaw Lair"),
        ("Toxullia Forest", "Toxxulia Forest"),
        ("The Deep", "Timorous Deep"),
        ("The Castle of Mistmoore", "Mistmoore Castle"),
    ];
    let resolves = |raw: &str| -> bool {
        if zones.iter().any(|z| z.name.eq_ignore_ascii_case(raw)) {
            return true;
        }
        if let Some(&(_, canon)) = aliases.iter().find(|&&(a, _)| a.eq_ignore_ascii_case(raw)) {
            if zones.iter().any(|z| z.name == canon) {
                return true;
            }
        }
        zones.iter().any(|z| {
            z.who_name
                .as_deref()
                .map(zonedata::map_shortnames)
                .unwrap_or_default()
                .iter()
                .any(|s| s.eq_ignore_ascii_case(raw))
        })
    };
    let mut unresolved: Vec<(&str, &str)> = Vec::new();
    for (spell, l) in teleportdata::all_landings() {
        if !resolves(&l.zone) {
            unresolved.push((spell, l.zone.as_str()));
        }
    }
    unresolved.sort();
    println!(
        "\nteleport landings whose destination is not a zonedata name: {} of {}",
        unresolved.len(),
        teleportdata::all_landings().count()
    );
    for (sp, z) in unresolved.iter().take(20) {
        println!("   {sp}  ->  {z:?}");
    }
    println!();

    let (mut no_class, mut no_level, mut not_known, mut ok) = (0, 0, 0, 0);
    let mut blocked_by_book: Vec<&str> = Vec::new();
    for (spell, l) in teleportdata::all_landings() {
        // why: the fixed gate -- the level of the port's OWN class
        let Some((_, have)) = per_class
            .iter()
            .find(|(c, _)| c.eq_ignore_ascii_case(l.class.as_str()))
        else {
            no_class += 1;
            continue;
        };
        if *have != 0 && *have < l.level {
            no_level += 1;
            continue;
        }
        if !known.is_empty() && !known.contains(&spell.to_ascii_lowercase()) {
            not_known += 1;
            blocked_by_book.push(spell);
            continue;
        }
        ok += 1;
    }
    println!(
        "teleports: wrong class {no_class}, under level {no_level}, NOT CAST IN THIS LOG {not_known}, usable {ok}"
    );
    blocked_by_book.sort();
    for s in blocked_by_book.iter().take(12) {
        println!("   gated by the spellbook gate: {s}");
    }
}
