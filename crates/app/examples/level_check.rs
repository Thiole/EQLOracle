//! why: why does the row say the level it says? Prints the class chain
//!      behind it, every floor the detector holds, the latest ding, and
//!      each /who row of your own.
//! input: <log>
use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;

fn main() {
    let path = std::env::args().nth(1).expect("log");
    let raw = std::fs::read(&path).expect("log");
    let lines = framed_lines(&raw);
    let engine = build_engine().expect("pack");
    let mut ing = Ingest::default();
    ing.character =
        eqlp_source::identity_from_filename(std::path::Path::new(&path)).map(|(c, _)| c);
    println!("character from the file name: {:?}", ing.character);
    for chunk in lines.chunks(100_000) {
        backfill_lines(&mut ing, &engine, chunk, 8);
    }
    let now = ing.now_ms();
    let you = ing.store.names.get("You").expect("You").0;
    println!("latest ding: {:?}", ing.levels.at(now));
    let chains = ing.classes.chains(you);
    println!("chains: {}", chains.len());
    for c in chains.iter().rev().take(4) {
        println!(
            "  chain {:?}..{:?} closed={:?} trio={:?} who={:?} floors={:?} max_ding={:?}",
            c.first,
            c.last,
            c.closed,
            c.trio(),
            c.who,
            c.floors,
            c.max_ding
        );
    }
    let cfg = ing.classes.configuration_of_visit(you, ing.unit_at(now));
    println!("current trio: {cfg:?}");
    println!(
        "you_level_at: {:?}",
        eqlp_app::combat::you_level_at(&ing, you, &cfg, now)
    );
}
