//! why: the map stayed blank in Oasis of Marr -- the wiki's who_name says
//! "marr", the game's map file is "oasis". Replays a real log and prints,
//! for the zone it ends in, what the app would actually load.
//! input: path to a real log
//! output: learned pair count, the end zone, its resolved map zones, and
//! which of those the install actually has a map file for
//! run: cargo run -p eqlp-app --release --example zone_map_check -- <log>
use eqlp_app::commands::map_zones_for_raw_label;
use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::mapsdata;
use eqlp_app::parser::build_engine;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: zone_map_check <log>");
    let raw = std::fs::read(&path).unwrap_or_else(|e| panic!("couldn't read {path}: {e}"));
    let lines = framed_lines(&raw);
    let engine = build_engine().expect("pack builds");
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let base = std::path::Path::new(&path)
        .parent()
        .and_then(|p| p.parent())
        .expect("<install>/Logs/<log>");

    let mut ing = Ingest::default();
    ing.set_spell_file(base);
    for chunk in lines.chunks(100_000) {
        backfill_lines(&mut ing, &engine, chunk, threads);
    }
    ing.mark_live();
    let ts = ing.now_ms();

    let have = mapsdata::list_all_zone_names(base);
    println!(
        "learned zone->shortname pairs: {}",
        ing.zone_shortnames.len()
    );
    println!("map files on disk: {}", have.len());

    let current = ing.zone.at(ts).map(str::to_string);
    println!("current zone: {current:?}");
    let zones = map_zones_for_raw_label(&ing, current.as_deref());
    println!("resolved map zones: {zones:?}");
    for z in &zones {
        println!(
            "  {z}: map file {}",
            if have.contains(z) { "YES" } else { "no" }
        );
    }
    if let Some((ts_ms, x, y, z)) = ing.last_loc {
        println!("last /loc: {ts_ms} ({x:.1}, {y:.1}, {z:.1})");
    }

    // why: every zone label the player has actually entered, resolved
    // through the real command path -- a zone with no map here is a zone
    // the map view goes blank in
    let verbose = std::env::var("EQLP_ZONE_VERBOSE").is_ok();
    if let Some(labels) = std::env::args().nth(2) {
        let text = std::fs::read_to_string(&labels).expect("labels file");
        let mut blank: Vec<&str> = Vec::new();
        let mut n = 0;
        for label in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
            n += 1;
            let zs = map_zones_for_raw_label(&ing, Some(label));
            let picked = zs.iter().find(|z| have.contains(*z));
            if picked.is_none() {
                blank.push(label);
            }
            if verbose {
                println!("  {label} -> {zs:?} picks {picked:?}");
            }
        }
        println!(
            "zones entered: {n}, resolving to a map: {}",
            n - blank.len()
        );
        for b in &blank {
            println!("  BLANK: {b}");
        }
    }

    // why: every zone the log ever states, so a second broken pairing
    // shows up here instead of in the next bug report
    let mut misses: Vec<String> = ing
        .zone_shortnames
        .iter()
        .filter(|(_, stem)| !have.contains(stem))
        .map(|(label, stem)| format!("{label} -> {stem}"))
        .collect();
    misses.sort();
    println!("learned pairs with no map file: {}", misses.len());
    for m in &misses {
        println!("  {m}");
    }
}
