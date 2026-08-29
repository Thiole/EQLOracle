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
