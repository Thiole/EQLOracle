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
