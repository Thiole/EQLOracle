use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;
use std::path::Path;

fn main() {
    let engine = build_engine().expect("pack builds");
    let path = Path::new(
        "/home/Spencer/Games/eq-legends/drive_c/users/Public/Daybreak Game Company/Installed Games/EverQuest Legends/Logs/eqlog_Manipulator_rivervale.txt",
    );
    let bytes = std::fs::read(path).expect("read log");
    let lines: Vec<&[u8]> = framed_lines(&bytes);
    let mut ing = Ingest::default();
    backfill_lines(&mut ing, &engine, &lines, lines.len());

    println!("confirmed turn-ins: {}", ing.turn_ins.len());
    for t in &ing.turn_ins {
        println!("  {} -> {:?}", t.who, t.items);
    }
}
