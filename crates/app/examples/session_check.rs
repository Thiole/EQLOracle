//! why: XP and AA read 0.0 in the overlay -- is that the data, or did the
//! warm-start startup break the hookup? A plain full fold answers it.
//! run: cargo run -p eqlp-app --release --example session_check -- <log>
use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::overview;
use eqlp_app::parser::build_engine;
fn main() {
    let path = std::env::args().nth(1).expect("usage: session_check <log>");
    let raw = std::fs::read(&path).unwrap();
    let lines = framed_lines(&raw);
    let engine = build_engine().unwrap();
    let mut ing = Ingest::default();
    ing.character = Some("Manipulator".to_string());
    if let Some(b) = std::path::Path::new(&path)
        .parent()
        .and_then(|p| p.parent())
    {
        ing.set_spell_file(b);
    }
    for chunk in lines.chunks(100_000) {
        backfill_lines(&mut ing, &engine, chunk, 24);
    }
    ing.mark_live();
    let s = overview::session(&ing);
    println!(
        "start={:?} end={:?}",
        ing.session_start(),
        ing.session_end()
    );
    println!(
        "duration_ms={:?}",
        ing.session_start()
            .map(|st| ing.session_end().saturating_sub(st))
    );
    println!(
        "aa_points rows={} aa_earned/hr={:?}",
        ing.aa_points.len(),
        s.aa_per_hour
    );
    println!(
        "xp_pct_per_hour={:?} levels_per_hour={:?}",
        s.xp_pct_per_hour, s.levels_per_hour
    );
    println!(
        "current_level={:?} platinum_per_hour={:?}",
        s.current_level, s.platinum_per_hour
    );
}
