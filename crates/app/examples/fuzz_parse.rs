//! why: adversarial -- does any hostile log line panic the parser
//! (which under the worker's lock would poison the ingest mutex)?
//! Feeds malformed/truncated/huge/unicode lines through the full
//! classify+apply path and reports any panic.
use eqlp_app::ingest::{backfill_lines, Ingest};
use eqlp_app::parser::build_engine;

fn try_line(engine: &eqlp_core::Engine, raw: &[u8], label: &str) {
    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut ing = Ingest::default();
        let lines = [raw];
        backfill_lines(&mut ing, engine, &lines, 1);
    }));
    if res.is_err() {
        println!(
            "PANIC on {label}: {:?}",
            String::from_utf8_lossy(&raw[..raw.len().min(80)])
        );
    }
}

fn main() {
    std::panic::set_hook(Box::new(|_| {})); // silence per-panic noise
    let engine = build_engine().expect("pack");
    let ts = "[Tue Jul 28 15:02:15 2026] ";
    let cases: Vec<(Vec<u8>, &str)> = vec![
        (b"".to_vec(), "empty"),
        (b"[".to_vec(), "lone bracket"),
        (ts.to_string().into_bytes(), "ts only"),
        (format!("{ts}You hit ").into_bytes(), "truncated hit"),
        (
            format!("{ts}You hit  for  points of damage.").into_bytes(),
            "empty fields",
        ),
        (
            format!("{ts}You hit X for 999999999999999999999999 points of damage.").into_bytes(),
            "huge number",
        ),
        (
            format!("{ts}You hit X for -5 points of damage.").into_bytes(),
            "negative dmg",
        ),
        (
            format!("{ts}{} slain", "A".repeat(200000)).into_bytes(),
            "200k name",
        ),
        (
            format!("{ts}\u{0}\u{0}\u{0} hits YOU").into_bytes(),
            "null bytes",
        ),
        (vec![0xff, 0xfe, 0xfd, b' '], "invalid utf8 leading"),
        (
            format!("{ts}\u{1F480}\u{1F600} has been slain by You!").into_bytes(),
            "emoji names",
        ),
        (
            format!("{ts}You are no longer ").into_bytes(),
            "truncated state",
        ),
        (
            format!("{ts}Your  spell has worn off of .").into_bytes(),
            "empty spell/target",
        ),
        (
            format!("{ts}You looted a  from a 's corpse.").into_bytes(),
            "empty loot",
        ),
        (format!("{ts}{}", "\t".repeat(5000)).into_bytes(), "tabs"),
        (
            format!("{ts}You gain experience! (999999.999%)").into_bytes(),
            "huge xp",
        ),
    ];
    for (raw, label) in &cases {
        try_line(&engine, raw, label);
    }
    // random fuzz
    let mut seed: u64 = 0x12345678;
    let mut rng = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for i in 0..20000u32 {
        let n = (rng() % 300) as usize;
        let mut b: Vec<u8> = ts.bytes().collect();
        for _ in 0..n {
            b.push((rng() % 256) as u8);
        }
        try_line(&engine, &b, &format!("fuzz#{i}"));
    }
    println!("done: 16 crafted + 20000 fuzz lines");
}
