//! why: the Group Buff Tracker recommended "Skin of the Shadow" (Kunark
//! Era, Necromancer 55) as an upgrade over Shield of Words -- this replays
//! a real log and asserts nothing it names is out of era or above the cap.
//! input: path to a real log
//! run: cargo run -p eqlp-app --release --example groupbuff_check -- <log>
use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;
use eqlp_app::{gearplanner, groupbuffs, spelldata};
use eqlp_session::classdetect::LEVEL_CAP;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: groupbuff_check <log>");
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
    // why: recommendations only exist while a party does -- an optional
    // "<Www Mmm DD HH:MM:SS YYYY>" replays to a moment one was grouped
    let until = std::env::args().nth(2).and_then(|a| {
        eqlp_core::header::by_name("bracket-ctime")
            .and_then(|h| h.parse(format!("[{a}] ").as_bytes()))
            .map(|(ts, _)| ts.secs() * 1000)
    });
    let h = eqlp_core::header::by_name("bracket-ctime").expect("header");
    for chunk in lines.chunks(100_000) {
        let cut = until.map_or(chunk.len(), |u| {
            chunk
                .iter()
                .position(|l| h.parse(l).is_some_and(|(ts, _)| ts.secs() * 1000 > u))
                .unwrap_or(chunk.len())
        });
        backfill_lines(&mut ing, &engine, &chunk[..cut], 24);
        if cut < chunk.len() {
            break;
        }
    }
    ing.mark_live();
    println!("party: {:?}", groupbuffs::group_buffs(&ing).party.len());

    let live = gearplanner::era_ix(gearplanner::CURRENT_ERA).expect("live era ranks");
    let dto = groupbuffs::group_buffs(&ing);
    let mut named = 0;
    let mut bad = 0;
    for kind in &dto.rows {
        for line in &kind.lines {
            named += 1;
            let Some(sp) = spelldata::spells()
                .iter()
                .find(|s| s.name == line.best_spell)
            else {
                continue;
            };
            let out_of_era = sp
                .era
                .as_deref()
                .and_then(gearplanner::era_ix)
                .is_some_and(|ix| ix > live);
            let over_cap = line.best_level > u32::from(LEVEL_CAP);
            if out_of_era || over_cap {
                bad += 1;
                println!(
                    "  UNREACHABLE {:<32} era={:?} level={} {}{}",
                    line.best_spell,
                    sp.era,
                    line.best_level,
                    if out_of_era { "[out of era]" } else { "" },
                    if over_cap { "[over cap]" } else { "" }
                );
            }
        }
    }
    println!("group buff lines named: {named}, unreachable: {bad}");
    println!("my_classes={:?}", dto.my_classes);
    for r in &dto.rows {
        println!(
            "  row {:<14} active={:?} upgrade={} best={:?}",
            r.label,
            r.active,
            r.upgrade,
            r.lines.first().map(|l| (&l.best_spell, l.best_level))
        );
        if r.active.is_none() {
            println!(
                "  MISSING {:<14} -> {}",
                r.label,
                r.lines
                    .iter()
                    .map(|l| l.best_spell.clone())
                    .take(3)
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    }
}
