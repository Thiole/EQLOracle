//! why: verify inventory::parse's exaltation tracking against a real full
//!      dump, not just synthetic fixtures
//! input: path to a real /outputfile inventory dump
//! output: every equip slot with real exalt sockets, printed
//! run: cargo run -p eqlp-app --example exalt_check -- <dump-path>

use eqlp_app::inventory::parse;
use std::path::Path;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: exalt_check <dump-path>");
    let parsed = parse(Path::new(&path)).expect("dump parses");
    let mut slots: Vec<&String> = parsed.exalted.keys().collect();
    slots.sort();
    for slot in slots {
        let sockets = &parsed.exalted[slot];
        let equipped = parsed
            .equipped
            .get(slot)
            .map(|i| i.name.as_str())
            .unwrap_or("?");
        println!("{slot:<10} ({equipped})");
        let mut keys: Vec<&String> = sockets.keys().collect();
        keys.sort();
        for k in keys {
            println!("  {k:<8} -> {}", sockets[k]);
        }
    }
    println!(
        "\n{} equip slots have real exalt sockets",
        parsed.exalted.len()
    );
}
