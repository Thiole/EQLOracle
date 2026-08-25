//! Tests for `classdetect`. Kept out of the production module by
//! convention: src/ contains shipping code only.

use eqlp_session::classdetect::Detector;

const YOU: u32 = 1;
const ALLY: u32 = 2;

const V1: Option<usize> = Some(1);
const V2: Option<usize> = Some(2);
const V3: Option<usize> = Some(3);
const V4: Option<usize> = Some(4);
const V5: Option<usize> = Some(5);

fn w(s: &str) -> Vec<String> {
    vec![s.to_string()]
}

/// The dominant (most zone visits) configuration's class list, or empty if
/// this entity has none.
fn dominant(d: &Detector, entity: u32) -> Vec<String> {
    d.configurations_of(entity)
        .into_iter()
        .next()
        .map(|(c, _)| c)
        .unwrap_or_default()
}

#[test]
fn a_cast_with_no_known_class_contributes_no_evidence() {
    let mut d = Detector::default();
    d.observe_cast(YOU, V1, &[]);
    assert!(d.configurations_of(YOU).is_empty());
}

#[test]
fn entities_are_tracked_independently() {
    let mut d = Detector::default();
    for v in [V1, V2] {
        d.observe_cast(YOU, v, &w("Wizard"));
        d.observe_cast(ALLY, v, &w("Cleric"));
    }
    assert_eq!(dominant(&d, YOU), vec!["Wizard"]);
    assert_eq!(dominant(&d, ALLY), vec!["Cleric"]);
    let mut known: Vec<u32> = d.known_entities().collect();
    known.sort();
    assert_eq!(known, vec![YOU, ALLY]);
}

#[test]
fn a_single_unambiguous_cast_does_not_confirm_on_its_own() {
    // See MIN_UNAMBIGUOUS_CASTS's own doc: one occasion isn't proof -- a
    // vendor-sold off-class spell, cast exactly once, must not read as a
    // confirmed class.
    let mut d = Detector::default();
    d.observe_cast(YOU, V1, &w("Magician"));
    assert!(
        d.configurations_of(YOU).is_empty(),
        "a lone unambiguous cast must not confirm anything yet"
    );
}

#[test]
fn a_second_occasion_confirms_the_class_retroactively() {
    // The visit that tips the count over confirms itself *and* every
    // earlier pending visit -- the first sighting isn't punished for
    // happening to be first.
    let mut d = Detector::default();
    d.observe_cast(YOU, V1, &w("Magician"));
    assert!(d.configuration_of_visit(YOU, V1).is_empty());
    d.observe_cast(YOU, V2, &w("Magician"));
    assert_eq!(
        d.configuration_of_visit(YOU, V1),
        vec!["Magician".to_string()]
    );
    assert_eq!(
        d.configuration_of_visit(YOU, V2),
        vec!["Magician".to_string()]
    );
}

#[test]
fn a_repeat_within_the_same_visit_is_still_only_one_occasion() {
    // Two casts of the same spell in the same visit is one occasion, not
    // two -- a burst/rapid-fire spam shouldn't cross the bar any faster
    // than a single cast would.
    let mut d = Detector::default();
    d.observe_cast(YOU, V1, &w("Magician"));
    d.observe_cast(YOU, V1, &w("Magician"));
    assert!(
        d.configurations_of(YOU).is_empty(),
        "same visit, twice, is still one occasion"
    );
}

#[test]
fn an_ambiguous_cast_never_confirms_a_new_class() {
    // "Alacrity" (Enchanter *and* Shaman) is ambiguous on its own -- with
    // neither candidate already confirmed this visit, it must add nothing.
    let mut d = Detector::default();
    d.observe_cast(YOU, V1, &["Enchanter".to_string(), "Shaman".to_string()]);
    assert!(
        d.configurations_of(YOU).is_empty(),
        "an ambiguous cast must not confirm either candidate on its own"
    );
}

#[test]
fn an_ambiguous_cast_only_reinforces_an_already_confirmed_candidate() {
    let mut d = Detector::default();
    for v in [V1, V2] {
        d.observe_cast(YOU, v, &w("Enchanter")); // unambiguous, confirms Enchanter
    }
    d.observe_cast(YOU, V1, &["Enchanter".to_string(), "Shaman".to_string()]); // ambiguous, same visit
    assert_eq!(
        dominant(&d, YOU),
        vec!["Enchanter"],
        "the ambiguous cast must not have confirmed Shaman too"
    );
}

#[test]
fn one_zone_visit_can_confirm_a_configuration_shorter_than_class_count() {
    // Nothing pads a visit's confirmed set up to CLASS_COUNT -- a visit
    // that only produced unambiguous evidence for one class stays at one.
    let mut d = Detector::default();
    d.observe_cast(YOU, V1, &w("Wizard"));
    d.observe_cast(YOU, V2, &w("Wizard"));
    assert_eq!(dominant(&d, YOU), vec!["Wizard"]);
}

#[test]
fn classes_confirmed_in_the_same_zone_visit_form_one_configuration() {
    let mut d = Detector::default();
    for v in [V1, V2] {
        d.observe_cast(YOU, v, &w("Wizard"));
        d.observe_cast(YOU, v, &w("Enchanter"));
        d.observe_cast(YOU, v, &w("Druid"));
    }
    let configs = d.configurations_of(YOU);
    assert_eq!(configs.len(), 1);
    assert_eq!(
        configs[0].0,
        vec!["Druid", "Enchanter", "Wizard"],
        "grouped alphabetically"
    );
    assert_eq!(configs[0].1, 2, "both zone visits used it");
}

#[test]
fn a_different_zone_visit_with_a_different_loadout_is_a_separate_configuration() {
    // The whole point: an occasional loadout swap doesn't overwrite or
    // evict the usual one, both are kept as distinct, real configurations.
    let mut d = Detector::default();
    for v in [V1, V2] {
        d.observe_cast(YOU, v, &w("Wizard"));
        d.observe_cast(YOU, v, &w("Enchanter"));
        d.observe_cast(YOU, v, &w("Druid"));
    }
    for v in [V3, V4] {
        d.observe_cast(YOU, v, &w("Shadow Knight"));
    }

    let configs = d.configurations_of(YOU);
    assert_eq!(configs.len(), 2);
    assert!(configs.iter().any(|(c, n)| c
        == &vec![
            "Druid".to_string(),
            "Enchanter".to_string(),
            "Wizard".to_string()
        ]
        && *n == 2));
    assert!(configs
        .iter()
        .any(|(c, n)| c == &vec!["Shadow Knight".to_string()] && *n == 2));
}

#[test]
fn repeated_visits_with_the_same_configuration_are_counted_not_deduplicated_away() {
    let mut d = Detector::default();
    for visit in [V1, V2, V3] {
        d.observe_cast(YOU, visit, &w("Wizard"));
        d.observe_cast(YOU, visit, &w("Enchanter"));
        d.observe_cast(YOU, visit, &w("Druid"));
    }
    // A one-off loadout still needs a second occasion to confirm at all --
    // give it one, on a different visit, so it's real evidence too.
    d.observe_cast(YOU, Some(4), &w("Shadow Knight"));
    d.observe_cast(YOU, Some(5), &w("Shadow Knight"));

    let configs = d.configurations_of(YOU);
    // Most zone visits first: the usual three-class loadout (3 visits)
    // outranks the rare Shadow Knight one (2 visits), but the latter is
    // still present, not evicted.
    assert_eq!(configs[0].0, vec!["Druid", "Enchanter", "Wizard"]);
    assert_eq!(configs[0].1, 3);
    assert_eq!(configs[1].0, vec!["Shadow Knight"]);
    assert_eq!(configs[1].1, 2);
}

#[test]
fn a_class_confirmed_before_any_zone_line_groups_under_the_none_visit() {
    // `None` (no `zone.enter` seen yet) is a legitimate grouping key on its
    // own, not an error case -- casts before the log's first zone line
    // still confirm real classes, on the same two-occasion terms as any
    // other visit (a repeat within `None` alone is still one occasion).
    let mut d = Detector::default();
    d.observe_cast(YOU, None, &w("Wizard"));
    d.observe_cast(YOU, V1, &w("Wizard"));
    assert_eq!(dominant(&d, YOU), vec!["Wizard"]);
    assert_eq!(
        d.configuration_of_visit(YOU, None),
        vec!["Wizard".to_string()]
    );
}

#[test]
fn configuration_of_visit_reports_only_that_visits_evidence() {
    let mut d = Detector::default();
    for v in [V1, V4] {
        d.observe_cast(YOU, v, &w("Wizard"));
    }
    for v in [V2, V4] {
        d.observe_cast(YOU, v, &w("Shadow Knight"));
    }
    assert_eq!(
        d.configuration_of_visit(YOU, V1),
        vec!["Wizard".to_string()]
    );
    assert_eq!(
        d.configuration_of_visit(YOU, V2),
        vec!["Shadow Knight".to_string()]
    );
    assert!(
        d.configuration_of_visit(YOU, V3).is_empty(),
        "a visit with no evidence yet reports nothing"
    );
}

fn m(classes: &[&str]) -> Vec<String> {
    classes.iter().map(|s| s.to_string()).collect()
}

/// Confirms Wizard and Enchanter for `V1` on two separate visits each
/// (crossing `MIN_UNAMBIGUOUS_CASTS`), then returns to `V1` so a test can
/// pick up the elimination logic from exactly `CLASS_COUNT - 1` confirmed,
/// same starting shape the old single-cast setup gave.
fn with_wizard_and_enchanter_confirmed() -> Detector {
    let mut d = Detector::default();
    for v in [V1, V2] {
        d.observe_cast(YOU, v, &w("Wizard"));
        d.observe_cast(YOU, v, &w("Enchanter"));
    }
    d
}

#[test]
fn elimination_confirms_the_third_class_from_two_different_ambiguous_pools() {
    // The real case this exists for: a Necromancer/Shadow-Knight character
    // whose only frequent evidence for that slot is spells shared between
    // the two (`Lifedraw`-shaped). With Wizard and Enchanter already
    // confirmed, `Lifedraw`'s pool alone can't resolve it -- but
    // `Ward of Calliav`'s pool (Beastlord/Magician/Necromancer) shares
    // only Necromancer with it, so the intersection lands on exactly one
    // class. Neither pool touches Wizard or Enchanter -- see this
    // function's sibling test for why a pool that does touch an
    // already-confirmed class can't be used this way.
    // why: narrowing to exactly one candidate from a single visit's own
    // pools is real evidence, but not proof by itself -- same real bug
    // this whole corroboration requirement exists for (see classdetect
    // module's own doc): broad pools can coincidentally intersect to a
    // wrong class. Elimination evidence specifically needs a 3rd,
    // independent distinct visit narrowing to the same class -- a
    // stricter bar than an unambiguous cast's own 2, since narrowing
    // is a much weaker signal (see MIN_ELIMINATION_CASTS's own doc).
    let mut d = with_wizard_and_enchanter_confirmed();
    // Exactly CLASS_COUNT - 1 confirmed now -- elimination can start.
    d.observe_cast(YOU, V1, &m(&["Necromancer", "Shadow Knight"])); // Lifedraw-shaped
    assert_eq!(
        d.configuration_of_visit(YOU, V1),
        m(&["Enchanter", "Wizard"]),
        "not resolved yet, only one pool seen"
    );

    d.observe_cast(YOU, V1, &m(&["Beastlord", "Magician", "Necromancer"])); // Ward of Calliav-shaped
    assert_eq!(
        d.configuration_of_visit(YOU, V1),
        m(&["Enchanter", "Wizard"]),
        "narrowed to Necromancer on just this one visit -- not proof by itself"
    );

    // A 2nd, distinct visit with the same two pools -- still not enough.
    d.observe_cast(YOU, V3, &w("Wizard"));
    d.observe_cast(YOU, V3, &w("Enchanter"));
    d.observe_cast(YOU, V3, &m(&["Necromancer", "Shadow Knight"]));
    d.observe_cast(YOU, V3, &m(&["Beastlord", "Magician", "Necromancer"]));
    assert_eq!(
        d.configuration_of_visit(YOU, V1),
        m(&["Enchanter", "Wizard"]),
        "narrowed to Necromancer on 2 visits now -- still not proof by itself"
    );

    // A 3rd, distinct visit finally corroborates it.
    d.observe_cast(YOU, V4, &w("Wizard"));
    d.observe_cast(YOU, V4, &w("Enchanter"));
    d.observe_cast(YOU, V4, &m(&["Necromancer", "Shadow Knight"]));
    d.observe_cast(YOU, V4, &m(&["Beastlord", "Magician", "Necromancer"]));
    assert_eq!(
        d.configuration_of_visit(YOU, V1),
        m(&["Enchanter", "Necromancer", "Wizard"]),
        "the intersection of both ambiguous pools narrows to exactly Necromancer, now corroborated"
    );
    assert_eq!(
        d.configuration_of_visit(YOU, V3),
        m(&["Enchanter", "Necromancer", "Wizard"])
    );
    assert_eq!(
        d.configuration_of_visit(YOU, V4),
        m(&["Enchanter", "Necromancer", "Wizard"])
    );
}

#[test]
fn a_pool_that_overlaps_an_already_confirmed_class_narrows_nothing() {
    // `Root` (Cleric/Enchanter/Necromancer/Paladin/Shaman/Wizard) includes
    // both already-confirmed classes -- it's fully explained by either one
    // alone, so it must be treated as pure reinforcement (this module's
    // first ambiguous-cast branch) and never reach narrowing, even though
    // Necromancer is also in its pool. Confirms the filter in
    // `elimination_confirms_the_third_class_from_two_different_ambiguous_pools`'s
    // sibling test is actually doing something, not just coincidentally
    // inert.
    let mut d = with_wizard_and_enchanter_confirmed();
    d.observe_cast(YOU, V1, &m(&["Necromancer", "Shadow Knight"])); // Lifedraw-shaped, narrowing = {Necromancer, Shadow Knight}
    d.observe_cast(
        YOU,
        V1,
        &m(&[
            "Cleric",
            "Enchanter",
            "Necromancer",
            "Paladin",
            "Shaman",
            "Wizard",
        ]),
    ); // Root-shaped
    assert_eq!(
        d.configuration_of_visit(YOU, V1),
        m(&["Enchanter", "Wizard"]),
        "Root-shaped evidence overlaps confirmed classes and must not narrow or confirm anything"
    );
}

#[test]
fn elimination_never_runs_with_more_than_one_slot_open() {
    // Only Wizard confirmed -- two slots open. Two ambiguous casts that
    // would narrow to Necromancer if only one slot were open must NOT
    // confirm anything here: either pool could belong to either open slot,
    // so intersecting them proves nothing.
    let mut d = Detector::default();
    for v in [V1, V2] {
        d.observe_cast(YOU, v, &w("Wizard"));
    }
    d.observe_cast(YOU, V1, &m(&["Necromancer", "Shadow Knight"]));
    d.observe_cast(
        YOU,
        V1,
        &m(&["Enchanter", "Magician", "Necromancer", "Wizard"]),
    );
    assert_eq!(
        d.configuration_of_visit(YOU, V1),
        vec!["Wizard".to_string()],
        "still just the one unambiguously confirmed class"
    );
}

#[test]
fn elimination_needs_a_second_pool_to_narrow_a_wide_first_one() {
    // A single wide ambiguous pool, even with one slot open, isn't enough
    // on its own -- there's nothing yet to intersect it against.
    let mut d = with_wizard_and_enchanter_confirmed();
    d.observe_cast(YOU, V1, &m(&["Cleric", "Druid", "Necromancer", "Shaman"]));
    let confirmed = d.configuration_of_visit(YOU, V1);
    assert_eq!(
        confirmed.len(),
        2,
        "a lone ambiguous pool of 4 candidates must not resolve anything by itself"
    );
}

#[test]
fn a_contradictory_pool_poisons_that_visits_narrowing_for_good() {
    // Two ambiguous pools sharing no class at all, both claiming the same
    // single open slot, can't both be right -- most likely a bad entry in
    // spell_classes.json for one of the spells involved, or (the real
    // incident this guards against) a genuine mid-visit reconfiguration.
    // See `narrow`'s own doc: a real contradiction poisons this visit's
    // elimination narrowing permanently rather than restarting from
    // whichever pool happened to arrive after it.
    let mut d = with_wizard_and_enchanter_confirmed();
    d.observe_cast(YOU, V1, &m(&["Necromancer", "Shadow Knight"]));
    d.observe_cast(YOU, V1, &m(&["Druid", "Shaman"])); // shares nothing with the above -- contradiction
    assert_eq!(
        d.configuration_of_visit(YOU, V1).len(),
        2,
        "the contradiction must not have confirmed anything"
    );

    // A pool that *would* have narrowed cleanly to Druid, had it arrived
    // first, must not un-poison this visit just because it arrives after.
    let druid_cleric = || {
        w("Druid")
            .into_iter()
            .chain(w("Cleric"))
            .collect::<Vec<_>>()
    };
    d.observe_cast(YOU, V1, &druid_cleric());
    assert_eq!(
        d.configuration_of_visit(YOU, V1),
        m(&["Enchanter", "Wizard"]),
        "poisoned -- must not silently pick Druid just because it showed up after the contradiction"
    );

    // 3 other, distinct visits independently narrow cleanly to Druid --
    // no contradiction on any of them -- crossing MIN_ELIMINATION_CASTS
    // and proving Druid globally.
    for v in [V3, V4, V5] {
        d.observe_cast(YOU, v, &w("Wizard"));
        d.observe_cast(YOU, v, &w("Enchanter"));
        d.observe_cast(YOU, v, &m(&["Druid", "Shaman"]));
        d.observe_cast(YOU, v, &druid_cleric());
    }
    for v in [V3, V4, V5] {
        assert_eq!(
            d.configuration_of_visit(YOU, v),
            m(&["Druid", "Enchanter", "Wizard"])
        );
    }
    // V1's own contradiction is permanent: proving Druid globally through
    // 3 *other* visits must not retroactively grant it to the visit whose
    // own evidence was self-contradictory.
    assert_eq!(
        d.configuration_of_visit(YOU, V1),
        m(&["Enchanter", "Wizard"]),
        "V1's own contradiction stays poisoned even after Druid is proven from other visits"
    );
}

#[test]
fn elimination_never_overrides_an_already_confirmed_class() {
    // Same boundary case as the plain reinforcement test, but exactly at
    // CLASS_COUNT - 1 confirmed, where elimination logic starts running --
    // an ambiguous cast that includes an already-confirmed candidate must
    // still just reinforce, never be treated as elimination evidence.
    let mut d = with_wizard_and_enchanter_confirmed();
    d.observe_cast(YOU, V1, &m(&["Enchanter", "Shaman"]));
    assert_eq!(
        d.configuration_of_visit(YOU, V1),
        m(&["Enchanter", "Wizard"]),
        "must not have touched narrowing at all"
    );
}

#[test]
fn a_partial_config_that_is_a_subset_of_exactly_one_full_config_gets_merged() {
    // "Enchanter" alone (a lagging visit, only one class confirmed so far)
    // is a subset of both Enchanter/Magician/Wizard AND Enchanter/Wizard --
    // wait, Enchanter/Wizard isn't full-length, so the only *full*
    // (CLASS_COUNT-length) candidate it's a subset of is
    // Enchanter/Magician/Wizard. Must merge into that one, not stand alone.
    let mut d = Detector::default();
    for v in [V1, V2] {
        d.observe_cast(YOU, v, &w("Wizard"));
        d.observe_cast(YOU, v, &w("Enchanter"));
        d.observe_cast(YOU, v, &w("Magician"));
    }
    d.observe_cast(YOU, V3, &w("Enchanter")); // partial: only one class confirmed this visit

    let (resolved, unresolved) = d.visits_by_resolved_configuration(YOU);
    assert!(
        unresolved.is_empty(),
        "the partial visit should have merged, not gone unresolved"
    );
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].0, vec!["Enchanter", "Magician", "Wizard"]);
    assert_eq!(
        resolved[0].1.len(),
        3,
        "the two full visits and the merged partial visit all count toward it"
    );
}

#[test]
fn a_partial_config_consistent_with_two_full_configs_is_reported_unresolved() {
    // "Wizard" alone is a subset of both Enchanter/Magician/Wizard and
    // Enchanter/Necromancer/Wizard -- genuinely ambiguous which one that
    // visit's incomplete evidence belongs to. Must not guess.
    let mut d = Detector::default();
    for v in [V1, Some(10)] {
        d.observe_cast(YOU, v, &w("Wizard"));
        d.observe_cast(YOU, v, &w("Enchanter"));
        d.observe_cast(YOU, v, &w("Magician"));
    }
    for v in [V2, Some(11)] {
        d.observe_cast(YOU, v, &w("Wizard"));
        d.observe_cast(YOU, v, &w("Enchanter"));
        d.observe_cast(YOU, v, &w("Necromancer"));
    }
    d.observe_cast(YOU, V3, &w("Wizard")); // ambiguous partial (Wizard already proven by now)

    let (resolved, unresolved) = d.visits_by_resolved_configuration(YOU);
    assert_eq!(
        resolved.iter().map(|(_, vs)| vs.len()).sum::<usize>(),
        4,
        "neither full config's count should have absorbed the ambiguous partial"
    );
    assert_eq!(unresolved, vec![V3]);
}

#[test]
fn a_partial_config_with_no_full_config_confirmed_at_all_is_unresolved() {
    let mut d = Detector::default();
    d.observe_cast(YOU, V1, &w("Wizard"));
    d.observe_cast(YOU, V2, &w("Wizard")); // crosses the threshold, never reaches 3 classes anywhere
    let (resolved, mut unresolved) = d.visits_by_resolved_configuration(YOU);
    assert!(resolved.is_empty());
    unresolved.sort();
    assert_eq!(unresolved, vec![V1, V2]);
}

#[test]
fn a_single_vendor_bought_off_class_spell_does_not_confirm_a_class() {
    // The real case MIN_UNAMBIGUOUS_CASTS exists for: a real Enchanter/
    // Wizard player who bought and cast "Protection of Wood" (wiki-
    // exclusive to Druid) exactly once, from a vendor that evidently
    // doesn't enforce class restrictions -- found against a real log.
    // One cast must not read as a confirmed third class.
    let mut d = with_wizard_and_enchanter_confirmed();
    d.observe_cast(YOU, V1, &w("Druid")); // "Protection of Wood"-shaped: one cast, one visit
    assert_eq!(
        d.configuration_of_visit(YOU, V1),
        m(&["Enchanter", "Wizard"]),
        "a single off-class cast must not promote to a third confirmed class"
    );
}

#[test]
fn a_class_can_be_swapped_back_to_at_any_time_with_no_boundary_needed() {
    // The whole reason this module doesn't try to model *when* a swap
    // happens: it can happen at will, in town, with zero log signal. A
    // class dropped from one visit's evidence can show up again in a later
    // visit with nothing special required to trigger it.
    let mut d = Detector::default();
    d.observe_cast(YOU, V1, &w("Wizard"));
    d.observe_cast(YOU, V2, &w("Necromancer"));
    d.observe_cast(YOU, V3, &w("Wizard")); // swapped back, and crosses the threshold
    assert!(d
        .configurations_of(YOU)
        .iter()
        .any(|(c, _)| c == &vec!["Wizard".to_string()]));
}
