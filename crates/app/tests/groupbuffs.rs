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
