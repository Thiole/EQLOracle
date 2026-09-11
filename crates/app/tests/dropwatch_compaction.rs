//! why: Drop Watch folds loot incrementally from a row watermark, and the
//! store compacts on every zone line. A watermark is a position, and
//! compaction moves positions.

use eqlp_app::dropwatch::loot_status;
use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;

fn run(log: &str) -> Ingest {
    let engine = build_engine().expect("pack builds");
    let lines = framed_lines(log.as_bytes());
    let mut ing = Ingest::default();
    backfill_lines(&mut ing, &engine, &lines, 1);
    ing.tick(ing.now_ms());
    ing
}

/// why: the real sequence -- Drop Watch polls WHILE the log grows, so the
/// row watermark is set before the zone line that compacts the store, not
/// after. A watermark is a position, and compaction moves positions.
#[test]
fn a_poll_before_a_zone_line_does_not_hide_the_loot_after_it() {
    let engine = build_engine().expect("pack builds");
    let mut ing = Ingest::default();
    let feed = |ing: &mut Ingest, text: &str| {
        let lines = framed_lines(text.as_bytes());
        backfill_lines(ing, &engine, &lines, 1);
        ing.tick(ing.now_ms());
    };
    let mut early = String::new();
    for i in 0..40 {
        early.push_str(&format!(
            "[Tue Jul 28 15:0{}:0{} 2026] You hit a gnoll for 10 points of fire damage by Burst of Flame.\n",
            i / 10,
            i % 10
        ));
    }
    early.push_str("[Tue Jul 28 15:05:00 2026] You have slain a gnoll!\n");
    early
        .push_str("[Tue Jul 28 15:05:05 2026] --You have looted a Jade from a gnoll's corpse.--\n");
    feed(&mut ing, &early);
    let watched = ["Jade".to_string()];
    let first = loot_status(&mut ing, &watched)
        .first()
        .map(|r| r.count)
        .unwrap_or(0);
    let len_before = ing.store.len();
    assert_eq!(first, 1, "premise: the first poll saw one Jade");

    feed(
        &mut ing,
        "[Tue Jul 28 15:30:00 2026] You have entered The Feerrott.\n",
    );
    let len_after = ing.store.len();
    feed(
        &mut ing,
        "[Tue Jul 28 15:30:10 2026] --You have looted a Jade from a rat's corpse.--\n",
    );
    let second = loot_status(&mut ing, &watched)
        .first()
        .map(|r| r.count)
        .unwrap_or(0);
    assert_eq!(
        second, 2,
        "both Jades. store rows {len_before} -> {len_after} across the zone line, \
         watermark was {len_before}"
    );
}

/// why: the count Drop Watch shows must equal the loot the log actually
/// recorded, whether or not a zone line compacted the store in between
#[test]
fn loot_after_a_zone_line_is_still_counted() {
    // why: real combat rows before the zone line -- those are what
    // compaction folds away, which is what moves every later row
    let mut log = String::from("[Tue Jul 28 15:00:00 2026] You have entered Blackburrow.\n");
    for i in 0..40 {
        log.push_str(&format!(
            "[Tue Jul 28 15:0{}:0{} 2026] You hit a gnoll for 10 points of fire damage by Burst of Flame.\n",
            i / 10,
            i % 10
        ));
    }
    log.push_str("[Tue Jul 28 15:05:00 2026] You have slain a gnoll!\n");
    log.push_str("[Tue Jul 28 15:05:05 2026] --You have looted a Jade from a gnoll's corpse.--\n");

    let before = {
        let mut ing = run(&log);
        loot_status(&mut ing, &["Jade".to_string()])
            .first()
            .map(|r| r.count)
            .unwrap_or(0)
    };
    assert_eq!(before, 1, "premise: one Jade looted before any zone line");

    // why: the zone line compacts, then a second Jade drops after it
    let after_zone = format!(
        "{log}[Tue Jul 28 15:30:00 2026] You have entered The Feerrott.\n\
         [Tue Jul 28 15:30:10 2026] --You have looted a Jade from a rat's corpse.--\n"
    );
    let mut ing = run(&after_zone);
    let counted = loot_status(&mut ing, &["Jade".to_string()])
        .first()
        .map(|r| r.count)
        .unwrap_or(0);
    assert_eq!(
        counted, 2,
        "both Jades: the zone line must not hide the second"
    );
}
