//! why: an AoE lands one line per target, so N of one name in a single
//! instant proves N were up. This is the only signal in the log that
//! counts concurrent duplicates -- see Ingest::note_fanout.
//! run: cargo run -p eqlp-app --release --example instance_check -- <log>
use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;
fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: instance_check <log>");
    let raw = std::fs::read(&path).unwrap();
    let lines = framed_lines(&raw);
    let engine = build_engine().unwrap();
    let mut ing = Ingest::default();
    ing.keep_full_history = true;
    for chunk in lines.chunks(100_000) {
        backfill_lines(&mut ing, &engine, chunk, 24);
    }
    ing.mark_live();
    let mut rows: Vec<(String, u32)> = ing
        .instances
        .iter()
        .map(|(sym, (n, _))| (ing.store.name(*sym).to_string(), *n))
        .collect();
    rows.sort_by_key(|(name, n)| (std::cmp::Reverse(*n), name.clone()));
    println!("names ever proven multi-instance: {}", rows.len());
    for (name, n) in rows.iter().take(18) {
        println!("  x{n}  {name}");
    }
}
