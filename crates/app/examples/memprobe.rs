//! why: measure where post-parse memory actually lives after a full
//! backfill of a real log -- optimize the measured hogs, not guesses.
//! run: ... --release --example memprobe -- <log>

use eqlp_app::ingest::{self, Ingest};
use eqlp_app::parser::build_engine;

fn main() {
    let path = std::env::args().nth(1).expect("usage: memprobe <log>");
    let raw = std::fs::read(&path).expect("read log");
    let engine = build_engine().expect("pack builds");
    let lines = ingest::framed_lines(&raw);
    let mut ing = Ingest::default();
    for chunk in lines.chunks(100_000) {
        ingest::backfill_lines(&mut ing, &engine, chunk, 8);
    }
    drop(raw);

    let s = &ing.store;
    let rows = s.len();
    // per-row columnar bytes: ts8 kind1 actor4 target4 ability4 amount8 flags4 enc4 tier1
    println!("store rows: {rows}  columnar ~{} MB", rows * 38 / 1_000_000);
    let name_bytes: usize = (0..s.names.len())
        .map(|i| s.name(eqlp_store::Sym(i as u32)).len())
        .sum();
    println!(
        "interner: {} names, ~{} KB strings (x2 for map keys)",
        s.names.len(),
        name_bytes / 1000
    );
    println!("abilities: {}", s.abilities.len());
    println!("store encounters: {}", s.encounters.len());

    let (eff_entities, pings, eff_bytes) = ing.effects.stats();
    println!(
        "effects: {eff_entities} entities, {pings} pings, ~{} MB strings (+{} MB struct @~80B/ping)",
        eff_bytes / 1_000_000,
        pings * 80 / 1_000_000
    );

    println!("timeline transitions: {}", ing.timeline.len());

    let chat_n = ing.chat.guild().len()
        + ing.chat.party().len()
        + ing.chat.raid().len()
        + ing.chat.pm_threads().map(|_| 1).count();
    println!("chat messages (channels+threads): {chat_n}");

    let closed = &ing.encounters.closed;
    let closed_entities: usize = closed
        .iter()
        .map(|c| c.entities.len() + c.slain.len())
        .sum();
    let closed_bytes: usize = closed
        .iter()
        .flat_map(|c| c.entities.iter().chain(c.slain.iter()))
        .map(|n| n.len() + 24)
        .sum();
    println!(
        "graph closed fights: {}, entity strings: {closed_entities} (~{} KB)",
        closed.len(),
        closed_bytes / 1000
    );

    let ebe: usize = ing.entities_by_enc.values().map(|v| v.len()).sum();
    let ebe_bytes: usize = ing
        .entities_by_enc
        .values()
        .flat_map(|v| v.iter())
        .map(|n| n.len() + 24)
        .sum();
    println!(
        "entities_by_enc: {} encs, {ebe} names (~{} KB)",
        ing.entities_by_enc.len(),
        ebe_bytes / 1000
    );

    // process-level ground truth
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    for l in status.lines() {
        if l.starts_with("VmHWM") || l.starts_with("VmRSS") {
            println!("{l}");
        }
    }
}
