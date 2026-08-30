//! why: empirical Drop Watch probe against a real log AS OF a mid-fight
//! instant -- prints the engaged-mob rows, the target's allegiance/group
//! evidence, and the active zone, to separate "data gap" from "wrongly
//! read as ally" when a watched drop fails to alert.
//! run: cargo run -p eqlp-app --release --example dropwatch_check -- <log> <line_limit> <mob>
use eqlp_app::dropwatch;
use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;

fn main() {
    let mut args = std::env::args().skip(1);
    let usage = "usage: dropwatch_check <log> <line_limit> <mob>";
    let path = args.next().expect(usage);
    let limit: usize = args.next().expect(usage).parse().expect(usage);
    let mob = args.next().expect(usage);

    let engine = build_engine().expect("pack builds");
    let bytes = std::fs::read(&path).expect("read log");
    let mut lines: Vec<&[u8]> = framed_lines(&bytes);
    lines.truncate(limit);
    let mut ing = Ingest::default();
    backfill_lines(
        &mut ing,
        &engine,
        &lines,
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4),
    );

    let now = ing.now_ms();
    println!("zone at now: {:?}", ing.zone.at(now));
    println!("charm: {:?}", ing.charm);
    println!(
        "{mob:?}: allegiance={}, currently_grouped={}, evidence={:?}",
        ing.allegiance_at(&mob, now).name(),
        ing.groups.currently_grouped(&mob, now),
        ing.groups.evidence_for(&mob),
    );
    let rows = dropwatch::drop_watch(&ing);
    println!("drop_watch rows: {}", rows.len());
    for r in rows.iter().take(10) {
        println!(
            "  {} -> {} known drops: {:?}",
            r.mob,
            r.drops.len(),
            r.drops
        );
    }
}
