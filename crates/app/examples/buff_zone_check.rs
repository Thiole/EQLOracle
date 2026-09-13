//! why: "its clearing out information zone to zone even though the
//! combination didnt change" -- replays a real log in small chunks and
//! prints the Group Buff Tracker's state on either side of every zone
//! line the player crossed while grouped. A zone that was not a class
//! swap must not drop the confirmed party or the rows.
//! input: path to a real log
//! run: cargo run -p eqlp-app --release --example buff_zone_check -- <log>
use eqlp_app::groupbuffs;
use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: buff_zone_check <log>");
    let raw = std::fs::read(&path).unwrap_or_else(|e| panic!("couldn't read {path}: {e}"));
    let lines = framed_lines(&raw);
    let engine = build_engine().expect("pack builds");
    let mut ing = Ingest::default();
    if let Some(b) = std::path::Path::new(&path)
        .parent()
        .and_then(|p| p.parent())
    {
        ing.set_spell_file(b);
    }
    // why: the state one chunk ago -- what the zone line is measured
    // against. Keyed by the roster's own names: a member who actually
    // LEFT is not a zone bug, and the tracker is right to drop them.
    let mut prev: Option<(Vec<String>, usize, usize)> = None;
    let mut prev_detail: Vec<String> = Vec::new();
    let mut crossings = 0usize;
    let mut wipes = 0usize;
    for chunk in lines.chunks(400) {
        let zoned = chunk
            .iter()
            .any(|l| String::from_utf8_lossy(l).contains("You have entered"));
        backfill_lines(&mut ing, &engine, chunk, 8);
        let d = groupbuffs::group_buffs(&ing, &[], None);
        let confirmed = d.party.iter().filter(|m| m.confirmed).count();
        let mut names: Vec<String> = d.party.iter().map(|m| m.name.clone()).collect();
        names.sort();
        let detail: Vec<String> = d
            .party
            .iter()
            .map(|m| format!("{} {:?} L{:?}", m.name, m.classes, m.level))
            .collect();
        let now = (names, confirmed, d.rows.len());
        if zoned {
            if let Some(before) = &prev {
                // why: the same roster on both sides -- a real join or
                // leave changes what the tracker can honestly say
                if !before.0.is_empty() && before.0 == now.0 {
                    crossings += 1;
                    let wiped = (before.1 > 0 && confirmed < before.1)
                        || (before.2 > 0 && d.rows.len() < before.2);
                    if wiped {
                        wipes += 1;
                        println!(
                            "zone: confirmed {}->{}  rows {}->{}\n  mine {:?}\n  was  {}\n  now  {}",
                            before.1, now.1, before.2, now.2, d.my_classes,
                            prev_detail.join(" | "), detail.join(" | ")
                        );
                    }
                }
            }
        }
        prev = Some(now);
        prev_detail = detail;
    }
    println!("\n{crossings} grouped zone crossings, {wipes} wiped the tracker");
    // why: the whole point of the soft cut -- a zone line is presence, not a swap
    assert_eq!(wipes, 0, "a zone line cleared the tracker");
}
