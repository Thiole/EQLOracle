//! why: Sync Check against the real install + log, not a fixture
//! input: <log> <base_dir>
use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::outputfiles::{sync_check, SyncRowDto};
use eqlp_app::parser::build_engine;
use std::path::Path;

fn main() {
    let mut a = std::env::args().skip(1);
    let log = a.next().expect("log path");
    let base = a.next().expect("base dir");
    let raw = std::fs::read(&log).unwrap_or_else(|e| panic!("couldn't read {log}: {e}"));
    let lines = framed_lines(&raw);
    let engine = build_engine().expect("pack builds");
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let mut ing = Ingest::default();
    for chunk in lines.chunks(100_000) {
        backfill_lines(&mut ing, &engine, chunk, threads);
    }
    println!("dump lines seen in log, by kind: {:?}\n", ing.dump_ts);
    let log_row = SyncRowDto {
        kind: "log".into(),
        label: "Combat log".into(),
        primary: true,
        status: "ok".into(),
        file: Some(
            Path::new(&log)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into(),
        ),
        modified_ms: None,
        dumped_at_ms: None,
        detail: "Tailing.".into(),
        command: Some("/log on".into()),
    };
    let out = sync_check(&ing.dump_ts, Some(Path::new(&base)), log_row);
    println!("OVERALL: {}\n", out.overall.to_uppercase());
    for r in &out.rows {
        println!(
            "  [{}] {:<14} {:<9} {}",
            if r.primary { "P" } else { "s" },
            r.label,
            r.status,
            r.file.as_deref().unwrap_or("-")
        );
        println!("       {}", r.detail);
    }
}
