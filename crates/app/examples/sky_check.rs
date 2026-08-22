//! why: verify skyquests::list_class_unlocks against the real Achievements
//!      dump + real inventory dump, not just synthetic fixtures
//! input: path to the game's base install folder
//! output: printed per-class unlock status + per-quest completion
//! run: cargo run -p eqlp-app --example sky_check -- <base_dir>

use eqlp_app::ingest::Ingest;
use eqlp_app::skyquests::list_class_unlocks;
use std::path::Path;

fn main() {
    let base_dir = std::env::args()
        .nth(1)
        .expect("usage: sky_check <base_dir>");
    let ing = Ingest::default();
    let classes = list_class_unlocks(&ing, Some(Path::new(&base_dir)));

    for c in &classes {
        let status = match c.unlocked {
            Some(true) => "UNLOCKED",
            Some(false) => "locked",
            None => "??? (no achievements dump found)",
        };
        println!("{:<14} {}", c.class, status);
        for r in &c.rewards {
            let done = match r.completed {
                Some(true) => "done",
                Some(false) => "open",
                None => "???",
            };
            println!("  [{done}] {:<28} (from {})", r.name, r.quest);
        }
    }
}
