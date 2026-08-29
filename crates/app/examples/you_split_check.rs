//! why: verify the "YOU" vs "You" identity split against a real log --
//! fold_key only folds the first char, so incoming-damage targets
//! ("... hits YOU for ...") may intern separately from "You".
//!
//! run: cargo run -p eqlp-app --release --example you_split_check -- <log>

use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: you_split_check <log>");
    let raw = std::fs::read(&path).expect("read log");
    let engine = build_engine().expect("pack builds");
    let lines = framed_lines(&raw);
    let mut ing = Ingest::default();
    backfill_lines(&mut ing, &engine, &lines, 8);

    println!("rows: {}", ing.store.len());
    for cand in ["You", "YOU", "you"] {
        match ing.store.names.get(cand) {
            Some(sym) => {
                let as_actor = ing.store.actor.iter().filter(|&&a| a == sym).count();
                let as_target = ing.store.target.iter().filter(|&&t| t == sym).count();
                println!(
                    "{cand:4} -> Sym({}) actor_rows={} target_rows={}",
                    sym.0, as_actor, as_target
                );
            }
            None => println!("{cand:4} -> not interned"),
        }
    }
}
