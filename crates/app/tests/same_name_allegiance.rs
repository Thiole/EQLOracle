//! why: a mob NAME is not an identity -- a zone holds many of one name,
//! and charm flips sides mid-fight. Shapes are from the real Lady Vox
//! raid at eqlog_Manipulator_rivervale.txt:667715.

use eqlp_app::combat::{list_allies, list_enemies, AllyDto};
use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};

/// why: rows the log cannot attribute to one of several same-named mobs.
/// Flagged at ingest, so a regression that starts guessing again shows up
/// as this number moving.
fn unresolved(ing: &Ingest) -> u64 {
    (0..ing.store.len())
        .filter(|&i| {
            ing.store.kind[i] == eqlp_store::EventKind::Damage
                && ing.store.flags[i] & eqlp_store::flag::UNRESOLVED_INSTANCE != 0
        })
        .map(|i| ing.store.amount[i])
        .sum()
}
use eqlp_app::parser::build_engine;

fn run(log: &str) -> Ingest {
    let engine = build_engine().expect("pack builds");
    let mut text = log.to_string();
    text.push_str(
        "[Tue Jul 28 15:01:59 2026] You hit a filler target for 1 points of fire damage by Burst of Flame.\n",
    );
    let lines = framed_lines(text.as_bytes());
    let mut ing = Ingest::default();
    backfill_lines(&mut ing, &engine, &lines, 1);
    ing.tick(ing.now_ms());
    ing
}

/// why: one printed shape for every case, so a change of behaviour is
/// read off the output instead of inferred from which assert tripped
fn split(ing: &Ingest, case: &str) -> (Vec<AllyDto>, Vec<AllyDto>) {
    let allies = list_allies(ing, None, None, false, None);
    let enemies = list_enemies(ing, None, None, false, None);
    println!("\n=== {case} ===");
    for a in &allies {
        let pets: Vec<String> = a
            .pets
            .iter()
            .map(|p| format!("{} {}", p.name, p.total))
            .collect();
        println!(
            "  ALLY  {:<34} {:>7}  pets[{}]",
            a.name,
            a.total,
            pets.join(", ")
        );
    }
    for e in &enemies {
        println!("  ENEMY {:<34} {:>7}", e.name, e.total);
    }
    (allies, enemies)
}

/// why: a charmed pet folds into its owner's row and is only listed
/// under `pets` -- counting top-level rows alone reports zero and reads
/// like a bug in the code rather than in the query
fn total_for(rows: &[AllyDto], needle: &str) -> u64 {
    rows.iter()
        .filter(|r| r.name.to_lowercase().contains(needle))
        .map(|r| r.total)
        .sum::<u64>()
        + rows
            .iter()
            .flat_map(|r| r.pets.iter())
            .filter(|p| p.name.to_lowercase().contains(needle))
            .map(|p| p.total)
            .sum::<u64>()
}

fn pets_of(rows: &[AllyDto], owner: &str) -> u64 {
    rows.iter()
        .filter(|r| r.name == owner)
        .flat_map(|r| r.pets.iter())
        .map(|p| p.total)
        .sum()
}

/// T1 -- the real Vox shape. You damage an ice giant, then charm one,
/// then the charmed one hits a DIFFERENT ice giant. Two individuals,
/// one name, opposite sides, one encounter.
#[test]
fn two_mobs_of_one_name_are_flagged_rather_than_guessed() {
    let ing = run(concat!(
        "[Tue Jul 28 15:01:00 2026] You hit an ice giant for 609 points of magic damage by Garrison's Mighty Mana Shock.\n",
        "[Tue Jul 28 15:01:02 2026] You begin casting Allure.\n",
        "[Tue Jul 28 15:01:05 2026] an ice giant has been charmed.\n",
        "[Tue Jul 28 15:01:08 2026] An ice giant hits an ice giant for 66 points of damage.\n",
        "[Tue Jul 28 15:01:10 2026] An ice giant hits an ice giant for 71 points of damage.\n",
    ));
    split(&ing, "T1 charmed + wild, same name");
    // why: one of the two giants is the pet and one is wild, and the line
    // names neither. Guessing would credit a wild mob's swing to the
    // player -- the row is marked instead.
    assert_eq!(unresolved(&ing), 137, "66 + 71, neither side claimed");
}

/// T2 -- the charm breaks mid-fight and the mob keeps swinging. The
/// pet-phase damage and the enemy-phase damage are the same name at two
/// different times; the split must follow the row's timestamp.
#[test]
fn a_charm_breaking_midfight_splits_that_name_by_time() {
    let ing = run(concat!(
        "[Tue Jul 28 15:01:00 2026] You begin casting Allure.\n",
        "[Tue Jul 28 15:01:03 2026] an ice giant has been charmed.\n",
        "[Tue Jul 28 15:01:06 2026] An ice giant hits a frost goblin for 500 points of damage.\n",
        "[Tue Jul 28 15:01:20 2026] Your Allure spell has worn off of an ice giant.\n",
        "[Tue Jul 28 15:01:24 2026] An ice giant hits YOU for 300 points of damage.\n",
        "[Tue Jul 28 15:01:26 2026] An ice giant hits YOU for 200 points of damage.\n",
    ));
    let (allies, enemies) = split(&ing, "T2 charm breaks mid-fight");
    assert_eq!(
        total_for(&allies, "ice giant"),
        500,
        "the 500 it dealt WHILE charmed stays on the ally side after the break"
    );
    assert_eq!(
        total_for(&enemies, "ice giant"),
        500,
        "the 500 it dealt after the break is enemy damage"
    );
}

/// T3 -- an ally casts the charm. `state.charm_broken` is self-only, so
/// no break line will ever arrive for this pet.
#[test]
fn an_ally_cast_charm_is_a_pet_without_any_break_line() {
    let ing = run(concat!(
        "[Tue Jul 28 15:00:50 2026] You have joined the group.\n",
        "[Tue Jul 28 15:00:51 2026] Sidhe has joined the group.\n",
        "[Tue Jul 28 15:01:00 2026] Sidhe begins casting Allure VI.\n",
        "[Tue Jul 28 15:01:03 2026] an ice giant has been charmed.\n",
        "[Tue Jul 28 15:01:06 2026] An ice giant hits a frost goblin for 400 points of damage.\n",
        "[Tue Jul 28 15:01:09 2026] Sidhe hits a frost goblin for 100 points of damage.\n",
    ));
    let (allies, _) = split(&ing, "T3 ally-cast charm, no break line exists");
    assert_eq!(
        pets_of(&allies, "Sidhe"),
        400,
        "the 400 belongs to Sidhe's charmed pet"
    );
}

/// T4 -- two casters hold the same NAME at once. Measured 56 times in
/// the real log (Manipulator + Sidhe on `an abhorrent`). These are two
/// individuals and must never share one bucket.
#[test]
fn one_casters_pet_rows_never_land_on_another_caster() {
    let ing = run(concat!(
        "[Tue Jul 28 15:01:00 2026] You begin casting Allure.\n",
        "[Tue Jul 28 15:01:03 2026] an ice giant has been charmed.\n",
        "[Tue Jul 28 15:00:50 2026] You have joined the group.\n",
        "[Tue Jul 28 15:00:51 2026] Sidhe has joined the group.\n",
        "[Tue Jul 28 15:01:05 2026] An ice giant hits a frost goblin for 400 points of damage.\n",
        "[Tue Jul 28 15:01:07 2026] Sidhe begins casting Allure VI.\n",
        "[Tue Jul 28 15:01:10 2026] an ice giant has been charmed.\n",
        "[Tue Jul 28 15:01:12 2026] An ice giant hits a frost goblin for 300 points of damage.\n",
        "[Tue Jul 28 15:01:15 2026] An ice giant hits a frost goblin for 300 points of damage.\n",
    ));
    let (allies, _) = split(&ing, "T4 two casters, one name, two pets");
    // why: the 400 landed while only your charm was held, so it is
    // provably yours. Everything after Sidhe's charm opened could be
    // either pet, so it is flagged -- and critically, none of it is
    // credited to Sidhe, which is what the one-slot-per-name keying did.
    assert_eq!(pets_of(&allies, "You"), 400, "only what is provably yours");
    assert_eq!(
        pets_of(&allies, "Sidhe"),
        0,
        "not one point of your pet's damage becomes Sidhe's"
    );
    assert_eq!(unresolved(&ing), 600, "the two ambiguous swings");
}

/// T5 -- a caster's second charm proves the first ended. One charm per
/// caster is a game rule; 14 cross-name switches in the real log.
#[test]
fn a_second_charm_by_one_caster_ends_that_casters_first() {
    let ing = run(concat!(
        "[Tue Jul 28 15:01:00 2026] You begin casting Allure.\n",
        "[Tue Jul 28 15:01:03 2026] an ice giant has been charmed.\n",
        "[Tue Jul 28 15:01:05 2026] An ice giant hits a frost goblin for 250 points of damage.\n",
        "[Tue Jul 28 15:01:08 2026] You begin casting Allure.\n",
        "[Tue Jul 28 15:01:11 2026] a frost goblin has been charmed.\n",
        "[Tue Jul 28 15:01:14 2026] An ice giant hits YOU for 400 points of damage.\n",
    ));
    let (allies, enemies) = split(&ing, "T5 second charm ends the first");
    assert_eq!(
        total_for(&enemies, "ice giant"),
        400,
        "the giant is no longer charmed once you charmed the goblin, so its 400 on you is enemy damage"
    );
    assert_eq!(
        total_for(&allies, "ice giant"),
        250,
        "its 250 from while it WAS charmed stays on the ally side"
    );
}

/// T6 -- two same-named mobs charmed by two DIFFERENT allies, both
/// swinging at one target. Neither is yours. The log line is byte-identical
/// for both pets, so nothing in the text separates them.
#[test]
fn two_allies_each_charm_one_name_and_both_hit_one_target() {
    let ing = run(concat!(
        "[Tue Jul 28 15:00:50 2026] You have joined the group.\n",
        "[Tue Jul 28 15:00:51 2026] Sidhe has joined the group.\n",
        "[Tue Jul 28 15:00:52 2026] Gibmund has joined the group.\n",
        "[Tue Jul 28 15:01:00 2026] Sidhe begins casting Allure VI.\n",
        "[Tue Jul 28 15:01:03 2026] an ice giant has been charmed.\n",
        "[Tue Jul 28 15:01:05 2026] An ice giant hits a frost goblin for 100 points of damage.\n",
        "[Tue Jul 28 15:01:07 2026] Gibmund begins casting Allure VI.\n",
        "[Tue Jul 28 15:01:10 2026] an ice giant has been charmed.\n",
        "[Tue Jul 28 15:01:12 2026] An ice giant hits a frost goblin for 200 points of damage.\n",
        "[Tue Jul 28 15:01:14 2026] An ice giant hits a frost goblin for 400 points of damage.\n",
        "[Tue Jul 28 15:01:16 2026] An ice giant hits a frost goblin for 800 points of damage.\n",
    ));
    let (allies, enemies) = split(&ing, "T6 two allies' pets, same name, one target");
    println!(
        "   Sidhe pets={}  Gibmund pets={}  goblin took={}",
        pets_of(&allies, "Sidhe"),
        pets_of(&allies, "Gibmund"),
        total_for(&enemies, "frost goblin")
    );
    // why: Sidhe's 100 landed while only her charm was held. The 1400
    // after Gibmund charmed could be either pet; before this keying it
    // ALL went to Gibmund, who had not earned a point of it.
    assert_eq!(pets_of(&allies, "Sidhe"), 100, "provably hers");
    assert_eq!(pets_of(&allies, "Gibmund"), 0, "none of hers becomes his");
    assert_eq!(unresolved(&ing), 1400, "ambiguous once both were held");
}

/// T7 -- a groupmate known only from group chat (no "has joined the
/// group" line, which is how Sidhe appears in the real log) fights
/// beside you, and the fight is read back an hour later. GROUP_TTL_MS
/// is 30 min, and `list_side` asks `currently_grouped` at NOW rather
/// than at the row's ts.
#[test]
fn a_past_groupmate_stays_an_ally_when_the_fight_is_read_back_later() {
    let ing = run(concat!(
        "[Tue Jul 28 15:01:00 2026] Sidhe tells the group, 'inc'\n",
        "[Tue Jul 28 15:01:02 2026] Sidhe hits a frost goblin for 900 points of damage.\n",
        "[Tue Jul 28 15:01:04 2026] You hit a frost goblin for 100 points of magic damage by Shock.\n",
        "[Tue Jul 28 15:01:06 2026] A frost goblin hits YOU for 50 points of damage.\n",
        // why: one hour later, well past GROUP_TTL_MS -- the fight above
        // is history now, exactly as the UI reads it after a session
        "[Tue Jul 28 16:05:00 2026] You hit a filler target for 1 points of fire damage by Burst of Flame.\n",
    ));
    let (allies, enemies) = split(&ing, "T7 groupmate, fight read back 1h later");
    assert_eq!(
        total_for(&allies, "sidhe"),
        900,
        "Sidhe was grouped DURING the fight, so her 900 is ally damage"
    );
    assert_eq!(
        total_for(&enemies, "sidhe"),
        0,
        "she must not appear as an enemy"
    );
}

/// T8 -- a player you are NOT grouped with, fighting the same mob in
/// the same zone. `Kind::Player` returns Ally unconditionally in
/// `allegiance_at`; the group check only guards `Kind::Unproven`.
#[test]
fn a_player_you_never_grouped_with_is_not_your_ally() {
    let ing = run(concat!(
        "[Tue Jul 28 15:01:00 2026] Sidhe shouts, 'anyone need a port'\n",
        "[Tue Jul 28 15:01:02 2026] Sidhe hits a frost goblin for 5000 points of damage.\n",
        "[Tue Jul 28 15:01:04 2026] You hit a frost goblin for 100 points of magic damage by Shock.\n",
        "[Tue Jul 28 15:01:06 2026] A frost goblin hits YOU for 50 points of damage.\n",
    ));
    let (allies, _) = split(&ing, "T8 ungrouped player, same mob");
    assert_eq!(
        total_for(&allies, "sidhe"),
        0,
        "never grouped, never buffed you -- her 5000 is not your group's damage"
    );
}

/// T9 -- a bard charms by singing. The game prints "Solon's Bewitching
/// Bravura"; the spell pack calls it "Solon's Bravura". Without the
/// alias `is_charm_spell` is false and the bard never owns the pet.
#[test]
fn a_bard_singing_charm_owns_the_pet() {
    let ing = run(concat!(
        "[Tue Jul 28 15:00:50 2026] You have joined the group.\n",
        "[Tue Jul 28 15:00:51 2026] Kaeus has joined the group.\n",
        "[Tue Jul 28 15:01:00 2026] Kaeus begins singing Solon's Bewitching Bravura VIII.\n",
        "[Tue Jul 28 15:01:03 2026] an ice giant has been charmed.\n",
        "[Tue Jul 28 15:01:06 2026] An ice giant hits a frost goblin for 640 points of damage.\n",
    ));
    let (allies, _) = split(&ing, "T9 bard charm via song");
    assert_eq!(
        pets_of(&allies, "Kaeus"),
        640,
        "the bard owns what he charmed"
    );
}

/// T10 -- the census: an AoE confirms how many of a name were standing at
/// that instant. A later cast sets the floor, never adds to it. A kill
/// after a confirmation means one fewer is left.
#[test]
fn an_aoe_sets_the_census_floor_and_a_kill_brings_it_down() {
    let census = |ing: &Ingest| -> Option<u32> {
        ing.store
            .names
            .get("a greater kobold")
            .and_then(|s| ing.instances.get(&s))
            .map(|(n, _)| *n)
    };
    // three mezzed in one instant
    let ing = run(concat!(
        "[Tue Jul 28 15:01:00 2026] a greater kobold has been mesmerized.\n",
        "[Tue Jul 28 15:01:00 2026] a greater kobold has been mesmerized.\n",
        "[Tue Jul 28 15:01:00 2026] a greater kobold has been mesmerized.\n",
    ));
    assert_eq!(census(&ing), Some(3), "one cast, three targets");

    // a second, smaller cast must NOT add -- and must not lower it either
    let ing = run(concat!(
        "[Tue Jul 28 15:01:00 2026] a greater kobold has been mesmerized.\n",
        "[Tue Jul 28 15:01:00 2026] a greater kobold has been mesmerized.\n",
        "[Tue Jul 28 15:01:00 2026] a greater kobold has been mesmerized.\n",
        "[Tue Jul 28 15:01:10 2026] a greater kobold has been mesmerized.\n",
        "[Tue Jul 28 15:01:10 2026] a greater kobold has been mesmerized.\n",
    ));
    assert_eq!(
        census(&ing),
        Some(3),
        "2 does not add to 3, and does not lower it"
    );

    // a bigger one raises it TO that number
    let ing = run(concat!(
        "[Tue Jul 28 15:01:00 2026] a greater kobold has been mesmerized.\n",
        "[Tue Jul 28 15:01:00 2026] a greater kobold has been mesmerized.\n",
        "[Tue Jul 28 15:01:10 2026] a greater kobold has been mesmerized.\n",
        "[Tue Jul 28 15:01:10 2026] a greater kobold has been mesmerized.\n",
        "[Tue Jul 28 15:01:10 2026] a greater kobold has been mesmerized.\n",
        "[Tue Jul 28 15:01:10 2026] a greater kobold has been mesmerized.\n",
    ));
    assert_eq!(census(&ing), Some(4), "raised to 4, not 2 + 4");

    // a kill after the confirmation leaves one fewer
    let ing = run(concat!(
        "[Tue Jul 28 15:01:00 2026] a greater kobold has been mesmerized.\n",
        "[Tue Jul 28 15:01:00 2026] a greater kobold has been mesmerized.\n",
        "[Tue Jul 28 15:01:00 2026] a greater kobold has been mesmerized.\n",
        "[Tue Jul 28 15:01:20 2026] A greater kobold has been slain by You!\n",
    ));
    assert_eq!(census(&ing), Some(2), "three confirmed, one died");
}
