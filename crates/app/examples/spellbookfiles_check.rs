//! why: verify spellbookfiles against a real character's real loadouts
//! run: cargo run -p eqlp-app --example spellbookfiles_check -- <install-dir> <file>

use eqlp_app::spellbookfiles::load_spellbook;
use std::path::Path;

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = args.next().expect("usage: <install-dir> <file>");
    let file = args.next().expect("usage: <install-dir> <file>");

    let sb = load_spellbook(Path::new(&dir), &file).unwrap_or_else(|e| panic!("load failed: {e}"));
    let in_use: Vec<_> = sb.loadouts.iter().filter(|l| l.in_use).collect();
    println!(
        "{} in-use loadouts (of {})",
        in_use.len(),
        sb.loadouts.len()
    );
    for lo in in_use {
        let filled = lo.slots.iter().filter(|s| s.spell_id != -1).count();
        let linked = lo.slots.iter().filter(|s| s.catalog_id.is_some()).count();
        let named = lo.slots.iter().filter(|s| s.name.is_some()).count();
        println!(
            "  #{:<2} {:<20} {filled:>2}/{} filled, {named}/{filled} named, {linked}/{filled} catalog-linked",
            lo.index,
            lo.name.clone().unwrap_or_default(),
            lo.slots.len(),
        );
        for s in &lo.slots {
            if s.spell_id != -1 && s.name.is_none() {
                println!("      !! unresolved id {} in slot {}", s.spell_id, s.slot);
            }
        }
    }
}
