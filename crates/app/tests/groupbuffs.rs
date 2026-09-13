//! why: one landing text is a whole line -- "A cool breeze slips through
//! your mind." is Clarity and Boon of the Clear Mind -- and Quick Buff
//! prints no cast line, so the ledger keeps both. Reported real: "Clarity
//! (over Boon of the Clear Mind)" nagging while Clarity was up.

use eqlp_app::groupbuffs::group_buffs;
use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;

fn run(log: &str) -> Ingest {
    let engine = build_engine().expect("pack builds");
    let lines = framed_lines(log.as_bytes());
    let mut ing = Ingest::default();
    ing.character = Some("Manipulator".to_string());
    backfill_lines(&mut ing, &engine, &lines, 1);
    ing.tick(ing.now_ms());
    ing
}

#[test]
fn a_quick_buff_landing_shared_by_a_line_reads_as_the_rank_worth_having() {
    let ing = run(concat!(
        "[Wed Sep 09 18:40:00 2026] You have entered The Northern Desert of Ro.\n",
        "[Wed Sep 09 18:40:05 2026] [60 ENC/WIZ/CLR] Manipulator (Human)  ZONE: The Northern Desert of Ro (nro)  \n",
        "[Wed Sep 09 18:54:41 2026] You activate Quick Buff.\n",
        "[Wed Sep 09 18:54:44 2026] A cool breeze slips through your mind.\n",
    ));
    let dto = group_buffs(&ing, &[], None);
    let row = dto
        .rows
        .iter()
        .find(|r| r.label == "mana regen")
        .unwrap_or_else(|| panic!("a mana regen row for an Enchanter: {dto:?}"));
    assert_eq!(row.active.as_deref(), Some("Clarity"), "row: {row:?}");
    assert!(
        !row.upgrade,
        "Clarity is up, nothing to cast over it: {row:?}"
    );
}

/// why: reported real -- "[Tue Aug 18 22:38:16 2026] Maenn has left the
/// group." must drop them from the roster and from Group Buffs, and keep
/// them out while they keep fighting the same mobs nearby
#[test]
fn an_explicit_leave_line_empties_the_party_and_later_shared_damage_does_not_undo_it() {
    let mut log = String::from(
        "[Tue Aug 18 10:00:00 2026] You have entered The Northern Desert of Ro.\n\
         [Tue Aug 18 10:00:05 2026] [60 ENC/WIZ/CLR] Manipulator (Human)  ZONE: The Northern Desert of Ro (nro)  \n\
         [Tue Aug 18 10:00:10 2026] Maenn has joined the group.\n\
         [Tue Aug 18 10:00:20 2026] Maenn begins casting Clarity.\n",
    );
    // why: a real standing groupmate -- gap-separated shared kills, so the
    // weak channel qualifies on its own history
    for d in 0..6 {
        let h = 10 + d * 3;
        log.push_str(&format!(
            "[Tue Aug 18 {h:02}:00:30 2026] Maenn hits a gnoll for 50 points of damage.\n\
             [Tue Aug 18 {h:02}:00:31 2026] You hit a gnoll for 10 points of fire damage by Burst of Flame.\n"
        ));
    }
    let ing = run(&log);
    let dto = group_buffs(&ing, &[], None);
    assert!(
        dto.party.iter().any(|p| p.name == "Maenn"),
        "premise: grouped and listed: {:?}",
        dto.party
    );

    let left = format!("{log}[Tue Aug 18 22:38:16 2026] Maenn has left the group.\n");
    let ing = run(&left);
    let now = ing.now_ms();
    assert!(
        !ing.groups.currently_grouped("Maenn", now),
        "the leave line drops membership"
    );
    let dto = group_buffs(&ing, &[], None);
    assert!(
        dto.party.is_empty(),
        "and Group Buffs stops listing them: {:?}",
        dto.party
    );

    let after = format!(
        "{left}[Tue Aug 18 22:38:46 2026] Maenn hits a gnoll for 50 points of damage.\n\
         [Tue Aug 18 22:38:47 2026] You hit a gnoll for 10 points of fire damage by Burst of Flame.\n"
    );
    let ing = run(&after);
    let dto = group_buffs(&ing, &[], None);
    assert!(
        dto.party.is_empty(),
        "still fighting the same mob is not rejoining: {:?}",
        dto.party
    );

    // why: the game's own word is the way back
    let rejoined = format!("{after}[Tue Aug 18 22:40:00 2026] Maenn has joined the group.\n");
    let ing = run(&rejoined);
    let dto = group_buffs(&ing, &[], None);
    assert!(
        dto.party.iter().any(|p| p.name == "Maenn"),
        "a rejoin line puts them back: {:?}",
        dto.party
    );
}

/// why: rule L10 -- a begin-cast is proof of the ABILITY to cast, so each
/// one raises the floor and nothing lowers it. Alacrity is ENC 21,
/// Augmentation ENC 28; a later cheaper cast must not walk it back.
#[test]
fn each_cast_raises_the_floor_and_a_cheaper_one_never_lowers_it() {
    let ing = run(concat!(
        "[Wed Sep 09 18:40:00 2026] You have entered The Northern Desert of Ro.\n",
        "[Wed Sep 09 18:40:05 2026] [60 ENC/WIZ/CLR] Manipulator (Human)  ZONE: The Northern Desert of Ro (nro)  \n",
        "[Wed Sep 09 18:40:10 2026] Maenn has joined the group.\n",
        "[Wed Sep 09 18:41:00 2026] Maenn begins casting Quickness.\n",
        "[Wed Sep 09 18:42:00 2026] Maenn begins casting Augmentation.\n",
        "[Wed Sep 09 18:43:00 2026] Maenn begins casting Alacrity.\n",
    ));
    let now = ing.now_ms();
    let (level, from_who) = ing.ally_level("Maenn", now);
    assert!(!from_who, "no /who row was printed for Maenn");
    assert_eq!(
        level,
        Some(28),
        "the highest cast proves 28; a later Alacrity (21) must not lower it"
    );
}

/// why: the floor is keyed to the CLASS that proved it, not to the zone
/// visit -- "the highest spell becomes the possible floor until new
/// evidence". A zone line is presence, not new evidence, and dropping the
/// floor there failed the rank gate for every line after it.
#[test]
fn a_floor_survives_a_zone_line() {
    let ing = run(concat!(
        "[Wed Sep 09 18:40:00 2026] You have entered The Northern Desert of Ro.\n",
        "[Wed Sep 09 18:40:05 2026] [60 ENC/WIZ/CLR] Manipulator (Human)  ZONE: The Northern Desert of Ro (nro)  \n",
        "[Wed Sep 09 18:40:10 2026] Maenn has joined the group.\n",
        "[Wed Sep 09 18:41:00 2026] Maenn begins casting Augmentation.\n",
        "[Wed Sep 09 19:00:00 2026] You have entered The Greater Faydark.\n",
    ));
    assert_eq!(
        ing.ally_level("Maenn", ing.now_ms()).0,
        Some(28),
        "walking through a zone line is not evidence they forgot the spell"
    );
}

/// why: Spencer -- "if its a multiclass spell, dont verify until classes
/// are verified". Alacrity is Enchanter 21 and Shaman 42; reading the
/// Shaman price off an unconfirmed guess put allies at 42 who were not
/// 40. Measured on the real log: 148 of 1114 peak floors came from a
/// class nobody had confirmed.
#[test]
fn a_multiclass_spell_proves_no_level_until_the_classes_are_verified() {
    let ing = run(concat!(
        "[Wed Sep 09 18:40:00 2026] You have entered The Northern Desert of Ro.\n",
        "[Wed Sep 09 18:40:05 2026] [60 ENC/WIZ/CLR] Manipulator (Human)  ZONE: The Northern Desert of Ro (nro)  \n",
        "[Wed Sep 09 18:40:10 2026] Maenn has joined the group.\n",
        "[Wed Sep 09 18:41:00 2026] Maenn begins casting Alacrity.\n",
    ));
    assert_eq!(
        ing.ally_level("Maenn", ing.now_ms()).0,
        None,
        "either class could have cast it, and neither is confirmed"
    );
}

/// why: the single-class half of the same rule -- only an Enchanter casts
/// Augmentation, so casting it verifies the class by itself. The
/// multiclass cast alongside it still contributes nothing.
#[test]
fn a_single_class_spell_proves_its_own_class_and_sets_the_floor() {
    let ing = run(concat!(
        "[Wed Sep 09 18:40:00 2026] You have entered The Northern Desert of Ro.\n",
        "[Wed Sep 09 18:40:05 2026] [60 ENC/WIZ/CLR] Manipulator (Human)  ZONE: The Northern Desert of Ro (nro)  \n",
        "[Wed Sep 09 18:40:10 2026] Maenn has joined the group.\n",
        "[Wed Sep 09 18:41:00 2026] Maenn begins casting Alacrity.\n",
        "[Wed Sep 09 18:42:00 2026] Maenn begins casting Augmentation.\n",
    ));
    assert_eq!(
        ing.ally_level("Maenn", ing.now_ms()).0,
        Some(28),
        "Augmentation is Enchanter 28; Alacrity's Shaman 42 is not theirs to claim"
    );
}

/// why: the spell tables carry Live's levels -- Clarity II is listed
/// Enchanter 54, above this server's cap of 50 -- and a requirement past
/// the cap says the row is wrong, not that the ally is max level
#[test]
fn a_requirement_above_the_level_cap_is_bad_data_and_sets_no_floor() {
    let ing = run(concat!(
        "[Wed Sep 09 18:40:00 2026] You have entered The Northern Desert of Ro.\n",
        "[Wed Sep 09 18:40:05 2026] [60 ENC/WIZ/CLR] Manipulator (Human)  ZONE: The Northern Desert of Ro (nro)  \n",
        "[Wed Sep 09 18:40:10 2026] Maenn has joined the group.\n",
        "[Wed Sep 09 18:41:00 2026] Maenn begins casting Clarity II.\n",
    ));
    assert_eq!(ing.ally_level("Maenn", ing.now_ms()).0, None);
}

/// why: rule L10's gate -- a class can be proven without any spell at all
/// (Harm Touch is Shadow Knight, no cast line, no rank), which leaves a
/// confirmed member with NO floor. That member caps at 0 and is credited
/// nothing; the old gate waived the check and handed out ranks to 50.
#[test]
fn a_class_proven_without_a_cast_has_no_floor_and_is_credited_nothing() {
    let mut log = String::from(
        "[Wed Sep 09 18:40:00 2026] You have entered The Northern Desert of Ro.\n\
         [Wed Sep 09 18:40:05 2026] [60 ENC/WIZ/CLR] Manipulator (Human)  ZONE: The Northern Desert of Ro (nro)  \n\
         [Wed Sep 09 18:40:10 2026] Maenn has joined the group.\n",
    );
    // why: repeated class-only evidence, spread over encounters, is what
    // clears the detector's bar -- one sighting is a guess
    for d in 0..6 {
        let h = 11 + d;
        log.push_str(&format!(
            "[Wed Sep 09 {h}:00:00 2026] Maenn begins to cast a spell.\n\
             [Wed Sep 09 {h}:00:05 2026] Maenn lets loose a Harm Touch on a rock golem for 300 points of damage.\n\
             [Wed Sep 09 {h}:00:09 2026] You have slain a rock golem!\n"
        ));
    }
    let ing = run(&log);
    let now = ing.now_ms();
    let (level, _) = ing.ally_level("Maenn", now);
    let dto = group_buffs(&ing, &[], None);
    let credited: Vec<(&str, u32)> = dto
        .rows
        .iter()
        .flat_map(|r| r.lines.iter())
        .filter(|l| l.casters.iter().any(|c| c.eq_ignore_ascii_case("Maenn")))
        .map(|l| (l.best_spell.as_str(), l.best_level))
        .collect();
    assert_eq!(level, None, "no cast and no /who row means no floor");
    assert!(
        credited.iter().all(|&(_, lvl)| lvl == 0),
        "ranks credited to a member with no proven level: {credited:?} party={:?}",
        dto.party
    );
}
