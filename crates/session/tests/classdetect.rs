//! why: docs/class-and-level-rules.md P1-P8, one test per rule shape.
//! Units are encounters (`Some(i)`); a class name alone is unambiguous
//! evidence, several names are a pool.

use eqlp_session::classdetect::{ChainEnd, Detector, Unit, CLASS_COUNT};

fn strs(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

fn cast(d: &mut Detector, unit: usize, classes: &[&str]) {
    d.observe_cast(1, Some(unit), &strs(classes));
}

fn trio(d: &Detector, unit: usize) -> Vec<String> {
    d.configuration_of_visit(1, Some(unit))
}

/// P2: one unit of unambiguous evidence is a candidate, two consecutive confirm
#[test]
fn unambiguous_evidence_confirms_on_the_second_consecutive_unit() {
    let mut d = Detector::default();
    cast(&mut d, 0, &["Wizard"]);
    assert!(trio(&d, 0).is_empty(), "one unit is not proof");
    cast(&mut d, 1, &["Wizard"]);
    assert_eq!(trio(&d, 1), strs(&["Wizard"]));
    // why: retroactive over the chain -- unit 0 reads the same
    assert_eq!(trio(&d, 0), strs(&["Wizard"]));
}

/// P2: elimination needs three units narrowing to the same class, and
/// only once two classes are already confirmed
#[test]
fn elimination_confirms_the_third_class_after_three_units() {
    let mut d = Detector::default();
    for u in 0..2 {
        cast(&mut d, u, &["Wizard"]);
        cast(&mut d, u, &["Enchanter"]);
    }
    for u in 2..5 {
        cast(&mut d, u, &["Wizard"]);
        cast(&mut d, u, &["Enchanter"]);
        cast(&mut d, u, &["Beastlord", "Cleric", "Druid"]);
        cast(&mut d, u, &["Cleric", "Paladin", "Shaman"]);
        if u < 4 {
            assert_eq!(trio(&d, u), strs(&["Enchanter", "Wizard"]), "unit {u}");
        }
    }
    assert_eq!(trio(&d, 4), strs(&["Cleric", "Enchanter", "Wizard"]));
    let view = d.chain_at(1, Some(4)).expect("chain");
    assert!(view.is_full());
    assert_eq!(view.candidates, Vec::<String>::new());
}

/// P2 as Spencer read it: the third slot's three units count from the
/// start, not from the moment the pair confirms -- pools seen in the
/// pair's own first two units are replayed against it
#[test]
fn elimination_counts_pools_from_before_the_pair_confirmed() {
    let mut d = Detector::default();
    for u in 0..3 {
        cast(&mut d, u, &["Wizard"]);
        cast(&mut d, u, &["Enchanter"]);
        cast(&mut d, u, &["Paladin", "Shadow Knight", "Warrior"]);
        cast(&mut d, u, &["Necromancer", "Shadow Knight"]);
    }
    assert_eq!(
        trio(&d, 2),
        strs(&["Enchanter", "Shadow Knight", "Wizard"]),
        "{:?}",
        d.chain_at(1, Some(2))
    );
}

/// Q34: the open slot reports what it is stuck between
#[test]
fn an_open_slot_reports_its_candidates() {
    let mut d = Detector::default();
    for u in 0..2 {
        cast(&mut d, u, &["Wizard"]);
        cast(&mut d, u, &["Enchanter"]);
    }
    cast(&mut d, 2, &["Wizard"]);
    cast(&mut d, 2, &["Enchanter"]);
    cast(&mut d, 2, &["Beastlord", "Cleric", "Druid"]);
    let view = d.chain_at(1, Some(2)).expect("chain");
    assert_eq!(view.candidates, strs(&["Beastlord", "Cleric", "Druid"]));
}

/// P1 as "all at once": a confirmed class stays confirmed through fights
/// that show nothing of it -- every trio holding it still fits the
/// whole chain; only a zone line (P4) or a contradiction (P5) moves it
#[test]
fn quiet_units_do_not_decay_a_confirmed_class() {
    let mut d = Detector::default();
    for u in 0..3 {
        cast(&mut d, u, &["Wizard"]);
        cast(&mut d, u, &["Enchanter"]);
    }
    for u in 3..9 {
        cast(&mut d, u, &["Wizard"]);
    }
    let view = d.chain_at(1, Some(8)).expect("chain");
    let mut confirmed = view.confirmed.clone();
    confirmed.sort();
    assert_eq!(confirmed, strs(&["Enchanter", "Wizard"]), "{view:?}");
    assert!(view.prior.is_empty());
}

/// P4: a zone line halves every weight -- the trio carries as a prior
/// until fresh evidence re-clears the bar
#[test]
fn a_zone_line_weakens_the_chain_without_breaking_it() {
    let mut d = Detector::default();
    for u in 0..2 {
        cast(&mut d, u, &["Wizard"]);
    }
    d.observe_zone_line(1, Some(2));
    cast(&mut d, 2, &["Enchanter", "Wizard"]);
    let view = d.chain_at(1, Some(2)).expect("chain");
    assert_eq!(view.prior, strs(&["Wizard"]), "{view:?}");
    assert!(view.confirmed.is_empty());
    cast(&mut d, 3, &["Wizard"]);
    let view = d.chain_at(1, Some(3)).expect("chain");
    assert_eq!(view.confirmed, strs(&["Wizard"]), "re-cleared: {view:?}");
    assert_eq!(d.chains(1).len(), 1, "one chain throughout");
}

/// P5: three consecutive conflicting units close the chain at the first
/// of them, and the new chain confirms on its own
#[test]
fn three_conflicting_units_close_the_chain_retroactively() {
    let mut d = Detector::default();
    for u in 0..3 {
        cast(&mut d, u, &["Wizard"]);
        cast(&mut d, u, &["Enchanter"]);
        cast(&mut d, u, &["Magician"]);
    }
    assert_eq!(trio(&d, 2), strs(&["Enchanter", "Magician", "Wizard"]));
    // a fourth class, three units running
    for u in 3..6 {
        cast(&mut d, u, &["Wizard"]);
        cast(&mut d, u, &["Enchanter"]);
        cast(&mut d, u, &["Druid"]);
    }
    let chains = d.chains(1);
    assert_eq!(chains.len(), 2, "{chains:?}");
    assert_eq!(chains[0].closed, Some(ChainEnd::Contradiction));
    assert_eq!(chains[0].last, Some(2), "closed where the conflict began");
    assert_eq!(chains[1].first, Some(3));
    assert_eq!(trio(&d, 5), strs(&["Druid", "Enchanter", "Wizard"]));
    assert_eq!(trio(&d, 2), strs(&["Enchanter", "Magician", "Wizard"]));
}

/// P5: a single conflicting unit is noise, not a close
#[test]
fn one_conflicting_unit_does_not_close_the_chain() {
    let mut d = Detector::default();
    for u in 0..3 {
        cast(&mut d, u, &["Wizard"]);
        cast(&mut d, u, &["Enchanter"]);
        cast(&mut d, u, &["Magician"]);
    }
    cast(&mut d, 3, &["Druid"]);
    // why: the view is chain-wide -- the run is 1 now, 0 once a clean unit follows
    assert_eq!(d.chain_at(1, Some(3)).expect("chain").conflicts, 1);
    cast(&mut d, 4, &["Wizard"]);
    assert_eq!(d.chains(1).len(), 1);
    assert_eq!(d.chain_at(1, Some(4)).expect("chain").conflicts, 0);
}

/// P8: a swap signal closes the chain now; the new chain starts empty
#[test]
fn a_swap_signal_closes_the_chain_immediately() {
    let mut d = Detector::default();
    for u in 0..2 {
        cast(&mut d, u, &["Wizard"]);
    }
    d.close_chain(1, Some(2));
    assert!(trio(&d, 2).is_empty());
    assert_eq!(trio(&d, 1), strs(&["Wizard"]));
    let chains = d.chains(1);
    assert_eq!(chains[0].closed, Some(ChainEnd::Swap));
}

/// P6: a ding raises every trio class below it and never lowers one; a
/// class confirming later in the chain picks up the chain's ding
#[test]
fn dings_raise_floors_and_never_lower_them() {
    let mut d = Detector::default();
    for u in 0..2 {
        cast(&mut d, u, &["Wizard"]);
        cast(&mut d, u, &["Enchanter"]);
    }
    d.observe_ding(1, Some(1), 50);
    for u in 2..4 {
        cast(&mut d, u, &["Wizard"]);
        cast(&mut d, u, &["Enchanter"]);
        cast(&mut d, u, &["Bard"]);
    }
    d.observe_ding(1, Some(3), 41);
    let view = d.chain_at(1, Some(3)).expect("chain");
    let floor = |c: &str| view.floors.iter().find(|(n, _)| n == c).map(|(_, l)| *l);
    assert_eq!(floor("Wizard"), Some(50));
    assert_eq!(floor("Enchanter"), Some(50));
    assert_eq!(
        floor("Bard"),
        Some(50),
        "confirmed later, picks up the chain's ding"
    );
}

/// P6: a spell only one trio class could cast raises that class to its
/// level; above the cap it proves nothing; a multi-class spell proves nothing
#[test]
fn spell_levels_raise_only_an_unambiguous_trio_class_under_the_cap() {
    let mut d = Detector::default();
    for u in 0..2 {
        cast(&mut d, u, &["Wizard"]);
        cast(&mut d, u, &["Enchanter"]);
    }
    d.observe_spell_levels(1, Some(2), &[("Wizard".to_string(), 43)]);
    d.observe_spell_levels(
        1,
        Some(2),
        &[("Wizard".to_string(), 55), ("Enchanter".to_string(), 50)],
    );
    d.observe_spell_levels(
        1,
        Some(2),
        &[("Enchanter".to_string(), 46), ("Cleric".to_string(), 30)],
    );
    cast(&mut d, 2, &["Wizard"]);
    let view = d.chain_at(1, Some(2)).expect("chain");
    let floor = |c: &str| view.floors.iter().find(|(n, _)| n == c).map(|(_, l)| *l);
    assert_eq!(floor("Wizard"), Some(43));
    assert_eq!(
        floor("Enchanter"),
        Some(46),
        "Cleric is not in the trio, so Enchanter alone fits"
    );
}

/// P7/C9 live in the app (pets never reach observe_cast); the detector
/// itself never exceeds CLASS_COUNT in a trio
#[test]
fn a_trio_never_exceeds_class_count() {
    let mut d = Detector::default();
    for u in 0..2 {
        for c in ["Wizard", "Enchanter", "Magician", "Druid"] {
            cast(&mut d, u, &[c]);
        }
    }
    assert!(trio(&d, 1).len() <= CLASS_COUNT);
}

/// why: chains split cleanly by units, so a unit query before the first
/// evidence finds nothing rather than a later chain
#[test]
fn a_unit_before_any_evidence_has_no_chain() {
    let mut d = Detector::default();
    cast(&mut d, 5, &["Wizard"]);
    cast(&mut d, 6, &["Wizard"]);
    let none: Unit = Some(2);
    assert!(d.chain_at(1, none).is_none());
    assert!(
        d.chain_at(1, Some(9)).is_some(),
        "the open chain covers what follows"
    );
}

/// why: once a zone is done a closed chain keeps only its result --
/// the trio, the floors and the unit list read the same after the
/// freeze, the evidence behind them is gone
#[test]
fn a_frozen_closed_chain_reads_the_same_as_before() {
    let mut d = Detector::default();
    for u in 0..3 {
        cast(&mut d, u, &["Wizard"]);
        cast(&mut d, u, &["Enchanter"]);
        cast(&mut d, u, &["Magician"]);
    }
    d.observe_ding(1, Some(2), 50);
    d.close_chain(1, Some(3));
    cast(&mut d, 3, &["Druid"]);
    let before = d.chain_at(1, Some(1)).expect("closed chain");
    let (cfg_before, _) = d.visits_by_resolved_configuration(1);
    d.freeze_closed(1);
    let after = d.chain_at(1, Some(1)).expect("frozen chain");
    assert_eq!(after.trio(), before.trio());
    assert_eq!(after.floors, before.floors);
    assert_eq!(after.closed, Some(ChainEnd::Swap));
    let (cfg_after, _) = d.visits_by_resolved_configuration(1);
    assert_eq!(cfg_after, cfg_before);
    // why: the open chain is untouched and still takes evidence
    cast(&mut d, 4, &["Druid"]);
    assert_eq!(trio(&d, 4), strs(&["Druid"]));
}
