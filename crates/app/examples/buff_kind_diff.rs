//! why: the game file's SPA now decides a buff's kind, the wiki prose
//! only when the file is silent. This lists every catalog spell where
//! the two disagree, so the switch is a reviewed list, not a hope.
//! input: the install folder (the one holding spells_us.txt)
//! run: cargo run -p eqlp-app --release --example buff_kind_diff -- <install>

use eqlp_app::groupbuffs::{is_party_buff, is_self_buff, kind_of, kind_of_with};
use eqlp_app::spelldata::spells;
use eqlp_app::spelltimers::spell_file;

fn main() {
    let base = std::env::args()
        .nth(1)
        .expect("usage: buff_kind_diff <install>");
    let file = spell_file(std::path::Path::new(&base));
    println!("file: {} spells", file.len());
    let (mut same, mut diff, mut unknown) = (0, 0, 0);
    // why: only what can reach the tracker -- a DoT or a slow is never a
    // party buff, and the build path gates on that before any kind
    for s in spells()
        .iter()
        .filter(|s| is_party_buff(s) || is_self_buff(s))
    {
        let text = kind_of(s);
        let game = kind_of_with(s, Some(&file));
        if eqlp_app::spelltimers::entry_of(&file, &s.name).is_none() {
            unknown += 1;
            continue;
        }
        if text == game {
            same += 1;
        } else {
            diff += 1;
            println!(
                "{:<34} prose={:<12} file={:?}",
                s.name,
                format!("{text:?}"),
                game
            );
        }
    }
    println!("\nsame={same} differ={diff} not-in-file={unknown}");
}
