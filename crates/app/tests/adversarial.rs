//! why: adversarial twins for the app suite's happy paths, driven by
//! real log-line shapes -- ordering variance the game genuinely
//! produces (death vs killing blow in either order, XP between kills,
//! same-second everything), conflicting data sources, and exact window
//! boundaries. A failure is a real bug or an unpinned semantic.

use eqlp_app::combat::list_encounters;
use eqlp_app::deathrecap::{death_timestamps, recap};
use eqlp_app::dropwatch::{drop_watch, loot_status};
use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;

fn run(log: &str) -> Ingest {
    let engine = build_engine().expect("pack builds");
    let lines = framed_lines(log.as_bytes());
    let mut ing = Ingest::default();
    backfill_lines(&mut ing, &engine, &lines, 1);
    ing
}

/// why: same trailing-filler + tick convention combat.rs's own
/// ingest_from uses -- a fight only idle-closes once a later line moves
/// the log clock past the 10s timeout, and slain/wiped are stamped at close
fn run_closed(log: &str) -> Ingest {
    let mut text = log.to_string();
    text.push_str(
        "[Tue Jul 28 15:01:30 2026] You hit a filler target for 1 points of fire damage by Burst of Flame.\n",
    );
    let mut ing = run(&text);
    ing.tick(ing.now_ms());
    ing
}

/// why: order variance -- the death line arriving BEFORE its own
/// killing blow within the same second. The kill must still be
/// confirmed; the trailing damage must not resurrect the fight into a
/// second phantom kill.
#[test]
fn a_death_line_arriving_before_its_killing_blow_still_confirms_one_kill() {
    let ing = run_closed(concat!(
        "[Tue Jul 28 15:01:00 2026] You hit a target 1 for 50 points of fire damage by Burst of Flame.\n",
        "[Tue Jul 28 15:01:05 2026] You have slain a target 1!\n",
        "[Tue Jul 28 15:01:05 2026] You hit a target 1 for 9 points of fire damage by Burst of Flame.\n",
    ));
    let list = list_encounters(&ing, None, 0, usize::MAX);
    let slain: Vec<_> = list
        .iter()
        .filter(|e| e.target == "a target 1" && e.slain)
        .collect();
    assert_eq!(slain.len(), 1, "exactly one confirmed kill, got {list:?}");
}

/// why: conflicting outcome -- the player dies AND the enemy's death is
/// confirmed in the same fight. Kill outranks wipe (a pyrrhic win is a
/// win); wiped must not be set.
#[test]
fn a_fight_where_both_you_and_the_enemy_die_reads_as_a_kill_not_a_wipe() {
    let ing = run_closed(concat!(
        "[Tue Jul 28 15:01:00 2026] a rock golem slashes You for 200 points of damage.\n",
        "[Tue Jul 28 15:01:01 2026] You hit a rock golem for 500 points of fire damage by Burst of Flame.\n",
        "[Tue Jul 28 15:01:02 2026] You have been slain by a rock golem!\n",
        "[Tue Jul 28 15:01:02 2026] You have slain a rock golem!\n",
    ));
    let list = list_encounters(&ing, None, 0, usize::MAX);
    let e = list
        .iter()
        .find(|e| e.target == "a rock golem")
        .expect("fight exists");
    assert!(e.slain, "the enemy died -- that's a kill");
    assert!(!e.wiped, "kill and wipe are mutually exclusive; kill wins");
}

/// why: identity conflict -- the log writes the player as You, YOU and
/// you across line shapes. All three must intern as ONE identity: the
/// recap totals only add up if incoming (YOU), heals (you) and deaths
/// (You) share a sym.
#[test]
fn all_three_player_casings_resolve_to_one_identity_in_the_recap() {
    let ing = run(concat!(
        "[Tue Jul 28 15:01:00 2026] Guard Fintran hits YOU for 25 points of damage.\n",
        "[Tue Jul 28 15:01:02 2026] Dippinsauce healed you for 40 hit points by Minor Healing.\n",
        "[Tue Jul 28 15:01:06 2026] You have been slain by Guard Fintran!\n",
    ));
    let r = recap(&ing, None).expect("death observed");
    assert_eq!(r.total_incoming, 25, "the YOU-cased hit must count");
    assert_eq!(r.total_healed, 40, "the you-cased heal must count");
}

/// why: exact window boundary -- a hit at EXACTLY death-30s. The scan
/// starts at partition_point(t < from), so t == from is IN. Pinned; an
/// off-by-one here silently drops the biggest early hit.
#[test]
fn a_hit_exactly_thirty_seconds_before_death_is_inside_the_recap_window() {
    let ing = run(concat!(
        "[Tue Jul 28 15:00:36 2026] Guard Fintran hits YOU for 111 points of damage.\n",
        "[Tue Jul 28 15:01:05 2026] Guard Fintran hits YOU for 25 points of damage.\n",
        "[Tue Jul 28 15:01:06 2026] You have been slain by Guard Fintran!\n",
    ));
    // death at 15:01:06; window from = 15:00:36 exactly
    let r = recap(&ing, None).expect("death observed");
    assert_eq!(
        r.total_incoming, 136,
        "the exactly-on-boundary 111 must be included"
    );
}

/// why: degenerate picker -- two deaths in the SAME second (real: a DoT
/// tick and a mob blow can both kill across a zone line glitch). The
/// list carries both; recap resolves without panic and pins to that
/// second.
#[test]
fn two_deaths_in_the_same_second_both_list_and_recap_cleanly() {
    let ing = run(concat!(
        "[Tue Jul 28 15:01:06 2026] You have been slain by Guard Fintran!\n",
        "[Tue Jul 28 15:01:06 2026] You have been slain by a rock golem!\n",
    ));
    let deaths = death_timestamps(&ing);
    assert_eq!(deaths.len(), 2);
    assert_eq!(deaths[0], deaths[1]);
    assert!(recap(&ing, Some(deaths[0])).is_some());
}

/// why: conflicting drop sources -- Eye of Veeshan carries "Efreeti
/// Great Staff" and "Fae Pauldrons" in BOTH monsters.json and
/// npcs.json known_loot. The union must dedupe, not list twice (the
/// overlay would render the duplicate).
#[test]
fn a_drop_named_by_both_catalogs_appears_once_not_twice() {
    let ing = run("[Tue Jul 28 15:01:00 2026] You hit Eye of Veeshan for 5 points of damage.\n");
    let rows = drop_watch(&ing);
    let eye = rows
        .iter()
        .find(|r| r.mob == "Eye of Veeshan")
        .expect("row exists");
    for item in ["Efreeti Great Staff", "Fae Pauldrons"] {
        let n = eye
            .drops
            .iter()
            .filter(|d| d.eq_ignore_ascii_case(item))
            .count();
        assert_eq!(
            n, 1,
            "{item} is in both catalogs -- must appear exactly once"
        );
    }
}

/// why: incremental-scan correctness under interleaving -- loot arriving
/// BETWEEN two loot_status calls must show in the second call with
/// totals identical to a from-scratch scan (the checkpoint fold and the
/// full rescan must never disagree)
#[test]
fn interleaved_loot_between_status_calls_matches_a_fresh_scan() {
    let engine = build_engine().expect("pack builds");
    let mut ing = Ingest::default();
    let first = "[Tue Jul 28 15:01:00 2026] --You have looted a Light Woolen Mask from a coyote's corpse.--\n";
    let second = "[Tue Jul 28 15:05:00 2026] --You have looted a Light Woolen Mask from a coyote's corpse.--\n";
    backfill_lines(&mut ing, &engine, &framed_lines(first.as_bytes()), 1);
    let items = vec!["Light Woolen Mask".to_string()];
    let r1 = loot_status(&mut ing, &items);
    assert_eq!(r1[0].count, 1);
    backfill_lines(&mut ing, &engine, &framed_lines(second.as_bytes()), 1);
    let r2 = loot_status(&mut ing, &items);
    assert_eq!(r2[0].count, 2, "checkpointed fold must see the new row");

    // from-scratch comparison
    let mut fresh = Ingest::default();
    backfill_lines(
        &mut fresh,
        &engine,
        &framed_lines(format!("{first}{second}").as_bytes()),
        1,
    );
    let rf = loot_status(&mut fresh, &items);
    assert_eq!(rf[0].count, r2[0].count);
    assert_eq!(rf[0].last_looted_ms, r2[0].last_looted_ms);
}

/// why: out-of-order zone lines -- a zone.enter with an EARLIER
/// timestamp arriving after a later one (loading-screen log variance).
/// Zone attribution must follow wall-clock order, not arrival order.
#[test]
fn an_out_of_order_zone_line_attributes_by_timestamp_not_arrival() {
    let ing = run(concat!(
        "[Tue Jul 28 15:02:00 2026] You have entered The Oasis of Marr.\n",
        "[Tue Jul 28 15:01:00 2026] You have entered Befallen.\n", // late arrival, earlier time
        "[Tue Jul 28 15:01:30 2026] You hit a rat for 5 points of damage.\n",
    ));
    assert_eq!(ing.zone.at(15 * 3600 + 90), ing.zone.at(15 * 3600 + 90)); // self-consistency
                                                                          // why: the fight at 15:01:30 belongs to Befallen's span, not Oasis's
    let list = list_encounters(&ing, None, 0, usize::MAX);
    let e = list
        .iter()
        .find(|e| e.target == "a rat")
        .expect("fight exists");
    let idx = ing.zone.index_at(e.start_ms).expect("in a zone");
    let label: Vec<_> = ing.zone.iter().collect();
    assert_eq!(
        label[idx].1, "Befallen",
        "timestamp order must win, got {label:?}"
    );
}

/// why: XP between two same-second kills -- pending_xp is consumed by a
/// same-timestamp death; with two candidate deaths in that second the
/// xp must attach to exactly one encounter, never both, never neither
#[test]
fn xp_between_two_same_second_kills_attaches_to_exactly_one() {
    let ing = run(concat!(
        "[Tue Jul 28 15:01:00 2026] You hit a target 1 for 50 points of fire damage by Burst of Flame.\n",
        "[Tue Jul 28 15:01:00 2026] You hit a target 2 for 50 points of fire damage by Burst of Flame.\n",
        "[Tue Jul 28 15:01:04 2026] You gain experience! (2.000%)\n",
        "[Tue Jul 28 15:01:04 2026] You have slain a target 1!\n",
        "[Tue Jul 28 15:01:04 2026] You have slain a target 2!\n",
    ));
    use eqlp_store::{EventKind, NO_ENCOUNTER};
    let attached: Vec<u32> = (0..ing.store.len())
        .filter(|&i| ing.store.kind[i] == EventKind::Xp)
        .map(|i| ing.store.enc[i])
        .collect();
    assert_eq!(attached.len(), 1);
    assert_ne!(attached[0], NO_ENCOUNTER, "the xp must attach to a kill");
}

/// why: a loot line for a mob killed by TIMEOUT (no death line at all,
/// ~79% of real kills) -- the loot row must still count for Drop
/// Watch's own totals even with no confirmed kill to hang it on
#[test]
fn loot_with_no_death_line_still_counts_in_loot_status() {
    let mut ing = run(concat!(
        "[Tue Jul 28 15:01:00 2026] You hit a coyote for 50 points of fire damage by Burst of Flame.\n",
        "[Tue Jul 28 15:20:00 2026] --You have looted a Light Woolen Mask from a coyote's corpse.--\n",
    ));
    let rows = loot_status(&mut ing, &["Light Woolen Mask".to_string()]);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].count, 1);
}

/// why: the reported "confirmed Bard for zones after I stopped being one".
/// A class only ever left the trio through a swap the parser had to
/// infer; the game says it outright in "The ability X is not available
/// to your class!", and that line was unparsed. Bard here is proven by
/// the one line only a Bard sees, then revoked.
#[test]
fn the_game_saying_an_ability_is_not_yours_takes_that_class_off_you() {
    // why: the bar is UNITS of class-only evidence, not lines, and a zone
    // line halves the weight carried across it -- three aura units clear
    // it, two do not
    let sung = concat!(
        "[Tue Jul 28 15:00:00 2026] You tell your party, 'ready'\n",
        "[Tue Jul 28 15:00:05 2026] This song cannot be played while Symphonic Aura is enabled.\n",
        "[Tue Jul 28 15:00:10 2026] You punch a target for 5 points of damage.\n",
        "[Tue Jul 28 15:00:15 2026] You have entered Lower Guk.\n",
        "[Tue Jul 28 15:00:20 2026] This song cannot be played while Symphonic Aura is enabled.\n",
        "[Tue Jul 28 15:00:25 2026] You punch a target for 5 points of damage.\n",
        "[Tue Jul 28 15:00:30 2026] You have entered Upper Guk.\n",
        "[Tue Jul 28 15:00:35 2026] This song cannot be played while Symphonic Aura is enabled.\n",
        "[Tue Jul 28 15:00:40 2026] You punch a target for 5 points of damage.\n",
    );
    let before = run(sung);
    let shown = |ing: &Ingest| {
        ing.class_chain("You", ing.now_ms())
            .map(|c| c.inferred())
            .unwrap_or_default()
    };
    assert!(
        shown(&before).iter().any(|c| c == "Bard"),
        "premise: the aura lines put Bard on the row -- got {:?}",
        shown(&before)
    );

    let revoked = run(&format!(
        // why: the chain closes at the NEXT unit, not mid-fight (P8) --
        // the zone line is what starts the clean one, which is exactly
        // the "for multiple zones after" in the report
        "{sung}[Tue Jul 28 15:00:45 2026] The ability Symphonic Aura: Disabled is not available to your class!\n[Tue Jul 28 15:00:50 2026] You have entered Befallen.\n[Tue Jul 28 15:00:55 2026] You punch a target for 5 points of damage.\n"
    ));
    assert!(
        !shown(&revoked).iter().any(|c| c == "Bard"),
        "the game said the class is not yours -- got {:?}",
        shown(&revoked)
    );
}

/// why: a bucket is any fights plus any log-time windows -- a window
/// counts only what fell inside it, a fight listed whole stays whole,
/// and per-ability dps runs on the bucket's combined fight time
#[test]
fn a_selection_clips_ranges_and_never_double_counts() {
    use eqlp_app::combat::{self, SelectionDto};
    let ing = run_closed(concat!(
        "[Tue Jul 28 15:01:00 2026] You hit a gnoll for 100 points of fire damage by Burst of Flame.\n",
        "[Tue Jul 28 15:01:03 2026] You hit a gnoll for 100 points of fire damage by Burst of Flame.\n",
        "[Tue Jul 28 15:01:05 2026] You have slain a gnoll!\n",
        "[Tue Jul 28 15:01:20 2026] You hit an orc for 100 points of fire damage by Burst of Flame.\n",
        "[Tue Jul 28 15:01:22 2026] You hit an orc for 100 points of fire damage by Burst of Flame.\n",
        "[Tue Jul 28 15:01:24 2026] You have slain an orc!\n",
    ));
    let t = |hms: &str| {
        let (h, m, sec) = (&hms[0..2], &hms[3..5], &hms[6..8]);
        let secs: i64 = h.parse::<i64>().unwrap() * 3600
            + m.parse::<i64>().unwrap() * 60
            + sec.parse::<i64>().unwrap();
        // why: the log clock is epoch ms; anchor on the gnoll fight's own start
        let gnoll = combat::list_encounters(&ing, None, 0, 50)
            .into_iter()
            .find(|e| e.target == "a gnoll")
            .expect("gnoll fight");
        gnoll.start_ms + (secs - (15 * 3600 + 60)) * 1000
    };
    // why: 15:01:02 .. 15:01:21 -- the gnoll's second hit and the orc's first
    let range = SelectionDto {
        encounters: vec![],
        ranges: vec![(t("15:01:02"), t("15:01:21"))],
        visits: vec![],
    };
    let s = combat::summarize(&ing, None, None, None, false, Some(&range));
    assert_eq!(s.fight_count, 2, "both fights touch the window");
    assert_eq!(s.total_damage, 200, "one hit from each fight inside it");
    assert_eq!(
        s.duration_ms,
        3_000 + 1_000,
        "gnoll :02-:05 plus orc :20-:21"
    );
    let bof = s
        .abilities
        .iter()
        .find(|a| a.ability == "Burst of Flame")
        .expect("row");
    assert!(
        (bof.dps - 200.0 / 4.0).abs() < 1e-9,
        "ability dps over the bucket's fight time"
    );

    let gnoll_id = combat::list_encounters(&ing, None, 0, 50)
        .into_iter()
        .find(|e| e.target == "a gnoll")
        .map(|e| e.id)
        .expect("gnoll id");
    let both = SelectionDto {
        encounters: vec![gnoll_id],
        ranges: vec![(t("15:01:02"), t("15:01:21"))],
        visits: vec![],
    };
    let s = combat::summarize(&ing, None, None, None, false, Some(&both));
    assert_eq!(s.fight_count, 2, "the gnoll fight is not counted twice");
    assert_eq!(
        s.total_damage, 300,
        "gnoll whole (200) plus the orc's clipped hit"
    );
    assert_eq!(s.duration_ms, 5_000 + 1_000);
    let allies = combat::list_allies(&ing, None, None, false, Some(&both));
    let you = allies.iter().find(|a| a.name == "You").expect("You row");
    assert_eq!(you.total, 300);

    // why: a whole visit as a member -- no zone line in this log, so
    // every fight sits in the "Unknown" visit (-1); a fight listed
    // whole and its visit together still count once
    let visit = SelectionDto {
        encounters: vec![gnoll_id],
        ranges: vec![],
        visits: vec![-1],
    };
    let s = combat::summarize(&ing, None, None, None, false, Some(&visit));
    // why: run_closed's own filler hit lands inside the orc fight's 6s
    // post-kill window, so it is part of that fight, not a third one
    assert_eq!(s.fight_count, 2);
    assert_eq!(s.total_damage, 401);
}

/// why: Spencer 2026-09-08 -- "harm touch or Reaving Strike (not reave)
/// is 100% a shadowknight confirmation". Real lines from the live log.
#[test]
fn harm_touch_and_reaving_strike_put_shadow_knight_on_the_row() {
    let shown = |ing: &Ingest, who: &str| {
        ing.class_chain(who, ing.now_ms())
            .map(|c| c.inferred())
            .unwrap_or_default()
    };
    let ht = run(concat!(
        "[Tue Jul 28 15:27:00 2026] Libaner tells the group, 'inc'\n",
        "[Tue Jul 28 15:27:49 2026] Libaner hit a dune spiderling for 12 points of magic damage by Harm Touch.\n",
    ));
    assert!(
        shown(&ht, "Libaner").iter().any(|c| c == "Shadow Knight"),
        "an ally's Harm Touch is a Shadow Knight -- got {:?}",
        shown(&ht, "Libaner")
    );
    let rs = run(concat!(
        "[Wed Jul 29 16:14:00 2026] Bravesirrobin tells the group, 'inc'\n",
        "[Wed Jul 29 16:14:56 2026] Bravesirrobin hit a boisterous gnoll for 40 points of magic damage by Reaving Strike.\n",
    ));
    assert!(
        shown(&rs, "Bravesirrobin")
            .iter()
            .any(|c| c == "Shadow Knight"),
        "an ally's Reaving Strike is a Shadow Knight -- got {:?}",
        shown(&rs, "Bravesirrobin")
    );
    let you = run(concat!(
        "[Sun Aug 16 09:17:00 2026] You tell your party, 'ready'\n",
        "[Sun Aug 16 09:17:48 2026] You begin casting Harm Touch X.\n",
        "[Sun Aug 16 09:17:48 2026] You hit Master of Spite for 921 points of unresistable damage by Harm Touch X.\n",
    ));
    assert!(
        shown(&you, "You").iter().any(|c| c == "Shadow Knight"),
        "your own Harm Touch is a Shadow Knight -- got {:?}",
        shown(&you, "You")
    );
    // why: Spencer's own shapes -- the ability has no cast line, the
    // damage line is the only first-person evidence there is
    let you_rs = run(concat!(
        "[Thu Jul 02 10:09:00 2026] You tell your party, 'ready'\n",
        "[Thu Jul 02 10:10:00 2026] You hit a sturdy skeleton for 34 points of magic damage by Reaving Strike.\n",
    ));
    assert!(
        shown(&you_rs, "You").iter().any(|c| c == "Shadow Knight"),
        "your own Reaving Strike is a Shadow Knight -- got {:?}",
        shown(&you_rs, "You")
    );
    // why: the gate -- a catalog spell on your own damage line is what a
    // weapon proc looks like, and must stay out of your detection
    let proc = run(concat!(
        "[Thu Jul 02 10:09:00 2026] You tell your party, 'ready'\n",
        "[Thu Jul 02 10:10:00 2026] You hit a sturdy skeleton for 34 points of magic damage by Ykesha.\n",
    ));
    assert!(
        shown(&proc, "You").is_empty(),
        "a catalog spell on your damage line is not evidence -- got {:?}",
        shown(&proc, "You")
    );
    let wipe = run(concat!(
        "[Thu Jul 02 07:47:00 2026] Wipe tells the group, 'inc'\n",
        "[Thu Jul 02 07:47:54 2026] Wipe hit a skeletal excavator for 30 points of magic damage by Reaving Strike.\n",
    ));
    assert!(
        shown(&wipe, "Wipe").iter().any(|c| c == "Shadow Knight"),
        "Wipe's Reaving Strike is a Shadow Knight -- got {:?}",
        shown(&wipe, "Wipe")
    );
}

/// why: Spencer -- Dragon Punch, Eagle Strike, Tiger Claw and Tail Rake
/// all print "strike", and Mend is Monk-only; both were invisible to
/// class detection (Mend was filed as noise, "strike" mapped to nothing).
/// Real lines from a Monk's own log.
#[test]
fn a_strike_a_frenzy_and_a_mend_each_put_their_one_class_on_the_row() {
    let shown = |ing: &Ingest, who: &str| {
        ing.class_chain(who, ing.now_ms())
            .map(|c| c.inferred())
            .unwrap_or_default()
    };
    let striker = run(concat!(
        "[Thu Jul 02 08:34:00 2026] Kaeus tells the group, 'inc'\n",
        "[Thu Jul 02 08:34:25 2026] Kaeus strikes skeleton L`rodd for 29 points of damage. (Critical)\n",
        "[Thu Jul 02 08:34:40 2026] Kaeus tries to strike skeleton L`rodd, but misses!\n",
    ));
    assert!(
        shown(&striker, "Kaeus").iter().any(|c| c == "Monk"),
        "an ally striking is a Monk -- got {:?}",
        shown(&striker, "Kaeus")
    );
    // why: the melee frenzy, not the spell -- real shapes, hit and parried
    let frenzier = run(concat!(
        "[Fri Aug 21 12:06:00 2026] Hemang tells the group, 'inc'\n",
        "[Fri Aug 21 12:06:21 2026] Hemang tries to frenzy on a froglok shin knight, but a froglok shin knight parries!\n",
        "[Fri Aug 21 12:06:25 2026] Hemang frenzies on a froglok shin knight for 52 points of damage.\n",
    ));
    assert!(
        shown(&frenzier, "Hemang").iter().any(|c| c == "Berserker"),
        "an ally frenzying is a Berserker -- got {:?}",
        shown(&frenzier, "Hemang")
    );
    let mender = run(concat!(
        "[Wed Jul 01 20:47:00 2026] You tell your party, 'ready'\n",
        "[Wed Jul 01 20:47:45 2026] You mend your wounds and heal some damage.\n",
    ));
    assert!(
        shown(&mender, "You").iter().any(|c| c == "Monk"),
        "mending is a Monk -- got {:?}",
        shown(&mender, "You")
    );
}

/// why: "Clarity needs to replace Boon of the clear mind, when clarity
/// is already active" -- one landing text for the whole line, and the
/// cast line a few seconds earlier says which rank it was
#[test]
fn a_shared_landing_text_resolves_to_the_rank_a_groupmate_just_cast() {
    let ing = run(concat!(
        "[Mon Sep 07 15:11:38 2026] Kilja begins casting Clarity.\n",
        "[Mon Sep 07 15:11:41 2026] A cool breeze slips through your mind.\n",
    ));
    let on_you: Vec<&String> = ing.self_buffs.keys().collect();
    assert!(
        on_you.iter().any(|n| n.as_str() == "Clarity"),
        "Clarity landed -- got {on_you:?}"
    );
    assert!(
        !on_you
            .iter()
            .any(|n| n.as_str() == "Boon of the Clear Mind"),
        "Boon was never cast -- got {on_you:?}"
    );
    // why: with no cast line to go on, every candidate stays -- the text
    // alone cannot tell them apart
    let blind = run("[Mon Sep 07 15:11:41 2026] A cool breeze slips through your mind.\n");
    assert!(
        blind.self_buffs.contains_key("Clarity")
            && blind.self_buffs.contains_key("Boon of the Clear Mind")
    );
}

/// why: "dont think it is detecting the shape for things like session
/// tracker with AA/hour" -- the single-point payout matched and went
/// nowhere, and its "1 ability point." tail did not even match
#[test]
fn every_ability_point_payout_shape_reaches_the_aa_ledger() {
    let ing = run(concat!(
        "[Sun Sep 06 21:14:11 2026] You have gained an ability point!  You now have 9 ability points.\n",
        "[Sun Sep 06 21:20:00 2026] You have gained an ability point!  You now have 1 ability point.\n",
        "[Sun Sep 06 21:25:00 2026] You have gained 2 ability point(s)!  You now have 3 ability point(s).\n",
    ));
    let gained: Vec<(u64, u64)> = ing.aa_points.iter().map(|&(_, g, t)| (g, t)).collect();
    assert_eq!(gained, vec![(1, 9), (1, 1), (2, 3)]);
}

/// why: "they leave and come back with same party member, with new
/// class loadouts, and it thinks the allies are the same loadout as
/// before". An ally proven Enchanter, then your zone line, then the
/// same ally only swinging -- no class line of theirs at all. The old
/// chain must not answer for the new visit.
#[test]
fn an_ally_seen_again_after_your_zone_line_does_not_keep_the_old_trio_on_swings_alone() {
    let before = concat!(
        "[Tue Jul 28 15:00:00 2026] Kilja tells the group, 'inc'\n",
        "[Tue Jul 28 15:00:05 2026] Kilja begins casting Clarity.\n",
        "[Tue Jul 28 15:00:08 2026] Kilja hits a gnoll for 5 points of damage.\n",
        "[Tue Jul 28 15:00:20 2026] You have entered Befallen.\n",
    );
    let ing = run(&format!(
        "{before}[Tue Jul 28 15:10:00 2026] Kilja hits a skeleton for 5 points of damage.\n[Tue Jul 28 15:10:02 2026] You hit a skeleton for 5 points of damage.\n"
    ));
    let now = ing.now_ms();
    let shown = ing
        .class_chain("Kilja", now)
        .map(|c| c.inferred())
        .unwrap_or_default();
    assert!(
        !shown.iter().any(|c| c == "Enchanter"),
        "the Enchanter chain is from before your zone line -- got {shown:?}"
    );
    // why: premise -- before the zone line the cast did prove Enchanter
    let old = run(before);
    let was = old
        .class_chain("Kilja", old.now_ms())
        .map(|c| c.inferred())
        .unwrap_or_default();
    assert!(
        was.iter().any(|c| c == "Enchanter"),
        "premise: Clarity proves Enchanter -- got {was:?}"
    );
}

/// why: "chain closed by a loadout swap signal -- why doesn't it show
/// the data it had at the time of that zone". A closed chain answers for
/// every fight up to the cut, not only up to its last class line, and a
/// presence cut is labelled as one, not as a swap.
#[test]
fn a_cut_chain_still_answers_for_the_fights_before_the_cut() {
    let ing = run(concat!(
        "[Tue Jul 28 15:00:00 2026] Kilja tells the group, 'inc'\n",
        "[Tue Jul 28 15:00:05 2026] Kilja begins casting Clarity.\n",
        "[Tue Jul 28 15:00:08 2026] Kilja hits a gnoll for 5 points of damage.\n",
        "[Tue Jul 28 15:00:09 2026] You hit a gnoll for 5 points of damage.\n",
        // why: a later fight in the same presence with no class line at all
        "[Tue Jul 28 15:01:00 2026] Kilja hits a rat for 5 points of damage.\n",
        "[Tue Jul 28 15:01:01 2026] You hit a rat for 5 points of damage.\n",
        "[Tue Jul 28 15:02:00 2026] You have entered Befallen.\n",
        "[Tue Jul 28 15:10:00 2026] Kilja hits a skeleton for 5 points of damage.\n",
        "[Tue Jul 28 15:10:01 2026] You hit a skeleton for 5 points of damage.\n",
    ));
    // why: the rat fight is 9 minutes and a second before the last line
    let rat_fight_ms = ing.now_ms() - 9 * 60_000 - 1_000;
    let at = ing
        .class_chain("Kilja", rat_fight_ms)
        .expect("the old chain covers the rat fight");
    assert!(
        at.inferred().iter().any(|c| c == "Enchanter"),
        "got {:?}",
        at.inferred()
    );
    assert_eq!(
        at.closed,
        Some(eqlp_session::classdetect::ChainEnd::Presence),
        "a presence cut, not a swap signal"
    );
    let now = ing
        .class_chain("Kilja", ing.now_ms())
        .map(|c| c.inferred())
        .unwrap_or_default();
    assert!(
        !now.iter().any(|c| c == "Enchanter"),
        "current detection started clean -- got {now:?}"
    );
}
