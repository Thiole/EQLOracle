//! Zone and session context, and grouping encounters by it.

use eqlp_session::{Context, Sessions, Spans};

#[test]
fn zone_is_a_query_not_a_stored_field() {
    let mut z = Spans::default();
    z.enter(1_000, "Nektulos Forest");
    z.enter(5_000, "Neriak - Foreign Quarter");
    assert_eq!(z.at(0), None, "before the first zone line");
    assert_eq!(z.at(1_000), Some("Nektulos Forest"));
    assert_eq!(z.at(4_999), Some("Nektulos Forest"));
    assert_eq!(z.at(5_000), Some("Neriak - Foreign Quarter"));
    assert_eq!(
        z.at(999_999),
        Some("Neriak - Foreign Quarter"),
        "holds until the next"
    );
}

#[test]
fn late_zone_lines_insert_in_position() {
    let mut z = Spans::default();
    z.enter(9_000, "Lower Guk");
    z.enter(1_000, "Befallen");
    assert_eq!(z.at(2_000), Some("Befallen"));
    assert_eq!(z.at(9_500), Some("Lower Guk"));
}

#[test]
fn re_entering_the_same_zone_is_a_new_span() {
    // why: player's own spec -- zoning out and straight back in is a new
    // visit. The old collapse rested on "zone lines repeat on load
    // screens", which the reference log disproves: 113 consecutive
    // same-zone enters, every one >=10s apart, zero duplicate prints --
    // each line is a real zoning (relog, camp, instance re-entry).
    let mut z = Spans::default();
    z.enter(1_000, "Befallen");
    z.enter(2_000, "Befallen");
    z.enter(3_000, "Befallen");
    assert_eq!(z.len(), 3);
    assert_ne!(z.index_at(1_500), z.index_at(2_500), "distinct visits");
    assert_eq!(z.at(1_500), z.at(2_500), "same zone name");
}

#[test]
fn revisiting_a_zone_is_a_distinct_visit() {
    // You enter Nektulos Forest 35 times in the reference log. Those are
    // separate visits, and a per-visit view must not merge them.
    let mut z = Spans::default();
    z.enter(1_000, "Nektulos Forest");
    z.enter(2_000, "Neriak - Foreign Quarter");
    z.enter(3_000, "Nektulos Forest");
    assert_eq!(z.len(), 3);
    assert_ne!(z.index_at(1_500), z.index_at(3_500), "different visits");
    assert_eq!(z.at(1_500), z.at(3_500), "same zone name");
}

#[test]
fn span_bounds_are_open_ended_at_the_last() {
    let mut z = Spans::default();
    z.enter(1_000, "A");
    z.enter(5_000, "B");
    assert_eq!(z.bounds(0), Some((1_000, Some(5_000))));
    assert_eq!(z.bounds(1), Some((5_000, None)));
    assert_eq!(z.bounds(2), None);
}

#[test]
fn label_before_gives_the_prior_zone_for_the_entrance_guess() {
    let mut z = Spans::default();
    z.enter(1_000, "West Commonlands");
    z.enter(5_000, "Befallen");
    z.enter(9_000, "Nektulos Forest");
    assert_eq!(z.label_before(1_500), None, "no zone before the first");
    assert_eq!(z.label_before(6_000), Some("West Commonlands"));
    assert_eq!(z.label_before(9_500), Some("Befallen"));
}

#[test]
fn sessions_are_inferred_from_silence_at_a_configurable_threshold() {
    let feed = |gap_ms: i64| {
        let mut s = Sessions::new(gap_ms);
        for t in [0i64, 1_000, 2_000, 700_000, 701_000, 3_000_000] {
            s.observe(t);
        }
        s.count()
    };
    assert_eq!(
        feed(600_000),
        3,
        "10min: two long gaps split three sessions"
    );
    assert_eq!(
        feed(6_000_000),
        1,
        "a threshold wider than every gap gives one"
    );
}

#[test]
fn grouping_by_zone_visit_separates_return_trips() {
    let mut c = Context::new(600_000);
    c.zone.enter(0, "Befallen");
    c.zone.enter(10_000, "Lower Guk");
    c.zone.enter(20_000, "Befallen");
    let encs = [(1u32, 1_000i64), (2, 5_000), (3, 12_000), (4, 25_000)];
    let by_visit = c.group_by_zone_visit(&encs);
    assert_eq!(by_visit.len(), 3, "three visits");
    assert_eq!(by_visit[0].2, vec![1, 2]);
    assert_eq!(by_visit[2].2, vec![4]);

    let by_name = c.group_by_zone_name(&encs);
    assert_eq!(by_name.len(), 2, "two distinct zone names");
    assert_eq!(by_name[0].1, vec![1, 2, 4], "both Befallen trips together");
}

#[test]
fn encounters_before_any_zone_line_are_grouped_as_unknown() {
    // Attaching to a log mid-session means no zone line has been seen yet.
    // That must be a labelled bucket, not a crash and not a wrong zone.
    let c = Context::new(600_000);
    let g = c.group_by_zone_name(&[(1u32, 5_000i64)]);
    assert_eq!(g[0].0, "unknown");
}

#[test]
fn grouping_by_session_works_on_the_same_shape() {
    let mut c = Context::new(600_000);
    for t in [0i64, 1_000, 900_000] {
        c.sessions.observe(t);
    }
    let g = c.group_by_session(&[(1u32, 500i64), (2, 950_000)]);
    assert_eq!(g.len(), 2);
    assert_eq!(g[0].0, "session-1");
    assert_eq!(g[1].0, "session-2");
}
