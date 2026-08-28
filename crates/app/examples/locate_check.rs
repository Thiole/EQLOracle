//! why: verify inventory::locate against a real dump directly -- this is
//! what found the real bag-content-swallowing bug (a bag's own first
//! content row, and every row after it in some bags, silently dropped
//! from `owned`/`locations`), before it ever reached the UI.
//! input: dump path, then any number of item names to locate
//! output: one block per name, its owned locations (or "0")
//! run: cargo run -p eqlp-app --release --example locate_check -- <dump> <item...>
use eqlp_app::inventory;
use std::path::Path;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: locate_check <dump> <item...>");
    let names: Vec<String> = args.collect();
    let parsed = inventory::parse(Path::new(&path)).expect("parse");
    println!("distinct owned items: {}", parsed.owned.len());
    for name in &names {
        let locs = parsed.locate(name);
        println!("{name}: {} location(s)", locs.len());
        for l in locs {
            println!("  {} tier{} x{}", l.label, l.tier, l.count);
        }
    }
}
