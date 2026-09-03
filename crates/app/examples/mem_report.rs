//! why: where the parsed state's memory actually goes -- a counting
//!      allocator, then each big field dropped in turn and the bytes it
//!      freed printed. Exact live bytes, not estimates.
//! input: <log>
//! run: cargo run -p eqlp-app --release --example mem_report -- <log>

use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Counting;
static LIVE: AtomicUsize = AtomicUsize::new(0);
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        LIVE.fetch_add(l.size(), Ordering::Relaxed);
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        LIVE.fetch_sub(l.size(), Ordering::Relaxed);
        unsafe { System.dealloc(p, l) }
    }
}
#[global_allocator]
static A: Counting = Counting;

fn mb(b: usize) -> f64 {
    b as f64 / 1_048_576.0
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let bytes = std::fs::read(&a[0]).expect("log readable");
    let lines = framed_lines(&bytes);
    let engine = build_engine().expect("pack builds");
    let base = LIVE.load(Ordering::Relaxed);
    let mut ing = Ingest::default();
    backfill_lines(&mut ing, &engine, &lines, 8);
    let total = LIVE.load(Ordering::Relaxed) - base;
    println!(
        "parsed state live: {:.1} MB ({} lines)",
        mb(total),
        lines.len()
    );
    let mut before = LIVE.load(Ordering::Relaxed);
    let mut report = |name: &str| {
        let now = LIVE.load(Ordering::Relaxed);
        println!("  {:<24} {:>8.1} MB", name, mb(before.saturating_sub(now)));
        before = now;
    };
    drop(std::mem::take(&mut ing.store.ts));
    drop(std::mem::take(&mut ing.store.kind));
    drop(std::mem::take(&mut ing.store.actor));
    drop(std::mem::take(&mut ing.store.target));
    drop(std::mem::take(&mut ing.store.ability));
    drop(std::mem::take(&mut ing.store.amount));
    drop(std::mem::take(&mut ing.store.flags));
    drop(std::mem::take(&mut ing.store.enc));
    drop(std::mem::take(&mut ing.store.tier));
    report("store event columns");
    drop(std::mem::take(&mut ing.store.encounters));
    report("store encounters");
    drop(std::mem::take(&mut ing.store.names));
    drop(std::mem::take(&mut ing.store.abilities));
    report("store names+abilities");
    drop(std::mem::take(&mut ing.effects));
    report("effects");
    drop(std::mem::take(&mut ing.chat));
    report("chat");
    drop(std::mem::take(&mut ing.timeline));
    report("timeline");
    drop(std::mem::take(&mut ing.spell_perf));
    report("spell_perf");
    drop(std::mem::take(&mut ing.groups));
    report("groups");
    drop(std::mem::take(&mut ing.zone));
    report("zone spans");
    drop(std::mem::take(&mut ing.classes));
    report("class detector");
    drop(std::mem::take(&mut ing.entities_by_enc));
    report("entities_by_enc");
    drop(std::mem::take(&mut ing.observed_drops));
    drop(std::mem::take(&mut ing.observed_zone_drops));
    report("observed drops");
    drop(std::mem::take(&mut ing.spell_ranks));
    drop(std::mem::take(&mut ing.spellbook));
    drop(std::mem::take(&mut ing.aa));
    drop(std::mem::take(&mut ing.exaltation_procs));
    drop(std::mem::take(&mut ing.levels));
    drop(std::mem::take(&mut ing.counts));
    drop(std::mem::take(&mut ing.turn_ins));
    drop(std::mem::take(&mut ing.recent));
    report("small logs (spells, aa, xp..)");
    drop(ing);
    report("everything else (graph, casts, ally chains, shapes..)");
}
