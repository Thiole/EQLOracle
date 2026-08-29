//! why: empirical check of the Game State party roster against a real
//! log -- reproduces debugview::game_state's own party build to see
//! where a reported four-thousand-member "party" actually comes from.
//! Also reports encounter involvement split (yours vs. someone else's).
//! run: cargo run -p eqlp-app --release --example gamestate_party_check -- <log> [line_limit]
//! line_limit: stop after that many lines -- checks the roster AS OF a
//! mid-log instant, not just the end
use eqlp_app::debugview;
use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: gamestate_party_check <log> [line_limit]");
    let limit: Option<usize> = std::env::args().nth(2).and_then(|s| s.parse().ok());
    let engine = build_engine().expect("pack builds");
    let bytes = std::fs::read(&path).expect("read log");
    let mut lines: Vec<&[u8]> = framed_lines(&bytes);
    if let Some(n) = limit {
        lines.truncate(n);
    }
    let mut ing = Ingest::default();
    backfill_lines(&mut ing, &engine, &lines, lines.len());

    let players: Vec<&str> = ing.encounters.entities.players().collect();
    println!("entities with Kind::Player (permanent):  {}", players.len());
    let dynamic = ing.groups.current_members(ing.now_ms());
    println!("GroupTracker current members (dynamic):  {}", dynamic.len());
    for (name, sessions, via, _) in &dynamic {
        println!("  dynamic: {name} sessions={sessions} via={}", via.name());
    }

    let gs = debugview::game_state(&ing);
    println!("game_state party total:                  {}", gs.party.len());
    println!("game_state known_players:                {}", gs.known_players);
    let mut by_via: std::collections::HashMap<&str, usize> = Default::default();
    for m in &gs.party {
        *by_via.entry(m.via).or_default() += 1;
    }
    println!("by via: {by_via:?}");
    for m in gs.party.iter().take(30) {
        println!("  {} via={} sessions={}", m.name, m.via, m.sessions);
    }

    let total = ing.store.encounters.len();
    let involved = ing.store.encounters.iter().filter(|e| e.involves_you).count();
    println!(
        "encounters: {total} total, {involved} involve you, {} someone else's",
        total - involved
    );
}
