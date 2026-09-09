//! why: the Debug db search is a contract -- newest first, the scan stops
//! at the limit, every table answers, and a bad regex is an error string

use eqlp_app::dbsearch::{civil, search, TABLES};
use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;

fn run(log: &str) -> Ingest {
    let engine = build_engine().expect("pack builds");
    let lines = framed_lines(log.as_bytes());
    let mut ing = Ingest::default();
    backfill_lines(&mut ing, &engine, &lines, 1);
    ing
}

fn three_hits() -> Ingest {
    run(concat!(
        "[Tue Jul 28 15:00:00 2026] You tell your party, 'ready'\n",
        "[Tue Jul 28 15:01:00 2026] You hit a gnoll for 10 points of damage.\n",
        "[Tue Jul 28 15:01:01 2026] You hit a gnoll for 20 points of damage.\n",
        "[Tue Jul 28 15:01:02 2026] You hit a rat for 30 points of damage.\n",
    ))
}

#[test]
fn events_come_newest_first_and_the_limit_cuts_the_scan() {
    let ing = three_hits();
    let all = search(&ing, "events", "", 100).expect("ok");
    assert_eq!(all.total, 3);
    assert_eq!(all.matched, 3);
    assert!(!all.truncated);
    assert_eq!(all.rows[0][6], "30", "newest first: {:?}", all.rows);
    let cut = search(&ing, "events", "", 1).expect("ok");
    assert_eq!(cut.rows.len(), 1);
    assert_eq!(cut.scanned, 1);
    assert!(cut.truncated);
}

#[test]
fn the_regex_runs_over_the_rendered_row_case_insensitively() {
    let ing = three_hits();
    let hit = search(&ing, "events", "GNOLL", 100).expect("ok");
    assert_eq!(hit.matched, 2, "{:?}", hit.rows);
    assert!(hit.rows.iter().all(|r| r[3] == "a gnoll"));
    let by_date = search(&ing, "events", "^2026-07-28 15:01:02", 100).expect("ok");
    assert_eq!(by_date.matched, 1);
    assert_eq!(by_date.rows[0][3], "a rat");
}

#[test]
fn a_bad_regex_or_table_is_an_error_not_a_panic() {
    let ing = three_hits();
    assert!(search(&ing, "events", "(", 100).is_err());
    assert!(search(&ing, "nope", "", 100).is_err());
}

#[test]
fn every_table_answers_on_small_and_empty_data() {
    let full = three_hits();
    let empty = Ingest::default();
    for t in TABLES {
        let dto = search(&full, t, "", 100).unwrap_or_else(|e| panic!("{t}: {e}"));
        assert_eq!(dto.table, *t);
        assert!(
            dto.rows.iter().all(|r| r.len() == dto.columns.len()),
            "{t}: ragged rows"
        );
        search(&empty, t, "", 100).unwrap_or_else(|e| panic!("{t} on empty: {e}"));
    }
}

#[test]
fn civil_matches_utc() {
    assert_eq!(civil(0), "1970-01-01 00:00:00");
    assert_eq!(civil(1_788_968_949_000), "2026-09-09 15:49:09");
}
