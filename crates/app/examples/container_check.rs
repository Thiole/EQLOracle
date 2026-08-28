//! why: verify inventory::parse's container browser view against a real dump
use eqlp_app::inventory;
use std::path::Path;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: container_check <dump>");
    let parsed = inventory::parse(Path::new(&path)).expect("parse");
    println!("{} containers", parsed.containers.len());
    for c in &parsed.containers {
        println!(
            "== {} ({}) -- {} slots ==",
            c.label,
            c.bag_item.as_deref().unwrap_or("no bag item"),
            c.slots.len()
        );
        for s in &c.slots {
            println!("  {}: {} +{} x{}", s.slot, s.item, s.tier, s.count);
        }
    }
}
