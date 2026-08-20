// Measures the live path: append -> poll -> frame -> classify.
use eqlp_core::{
    engine::Engine,
    event::Outcome,
    frame::Framer,
    rule::{Pack, ResolvedPack},
};
use eqlp_source::Tail;
use std::io::Write;
use std::time::Instant;

fn main() {
    let src = std::fs::read_to_string("packs/eql.toml").unwrap();
    let eng =
        Engine::build(&ResolvedPack::layer(vec![Pack::from_toml(&src).unwrap()]).unwrap()).unwrap();
    let mut m = eng.matcher();

    let real = std::fs::read("/mnt/user-data/uploads/eqlog_Manipulator_rivervale.txt").unwrap();
    let lines: Vec<&[u8]> = eqlp_core::frame::lines(&real).take(200_000).collect();

    let path = "/tmp/live.log";
    let _ = std::fs::remove_file(path);
    std::fs::write(path, b"").unwrap();
    let mut f = std::fs::OpenOptions::new().append(true).open(path).unwrap();

    let mut tail = Tail::from_start(path);
    let mut fr = Framer::default();
    let _ = tail.poll(|_| {});

    // --- 1. cost of a poll when nothing changed (the common case) ---
    let n = 10_000;
    let t0 = Instant::now();
    for _ in 0..n {
        tail.poll(|_| {});
    }
    let idle = t0.elapsed();
    println!(
        "idle poll (no change)   : {:>8.1} ns each  ({} polls)",
        idle.as_nanos() as f64 / n as f64,
        n
    );

    // --- 2. burst: write a chunk, measure detect+parse ---
    for &burst in &[1usize, 10, 100, 1000] {
        let mut wrote = 0usize;
        let mut total = std::time::Duration::ZERO;
        let rounds = 200;
        for r in 0..rounds {
            let start = (r * burst) % (lines.len() - burst);
            let mut buf = Vec::new();
            for l in &lines[start..start + burst] {
                buf.extend_from_slice(l);
                buf.push(b'\n');
            }
            wrote += buf.len();
            f.write_all(&buf).unwrap();
            f.flush().unwrap();

            let t = Instant::now();
            let mut parsed = 0u32;
            tail.poll(|chunk| {
                fr.push(chunk, |line| {
                    if let Outcome::Matched(_) = m.classify(line) {
                        parsed += 1;
                    }
                });
            });
            total += t.elapsed();
            std::hint::black_box(parsed);
        }
        println!(
            "burst {:>4} lines        : {:>8.1} µs per poll  ({:.0} ns/line, {} KiB total)",
            burst,
            total.as_micros() as f64 / rounds as f64,
            total.as_nanos() as f64 / (rounds * burst) as f64,
            wrote / 1024
        );
    }

    // --- 3. torn write: one byte at a time ---
    let line = b"[Wed Aug 06 21:14:33 2025] You slash a rat for 12 points of damage.\n";
    let mut emitted = 0;
    let t = Instant::now();
    for b in line.iter() {
        f.write_all(&[*b]).unwrap();
        f.flush().unwrap();
        tail.poll(|c| fr.push(c, |_| emitted += 1));
    }
    println!(
        "torn write ({} polls)   : {:>8.1} µs total, {} line(s) emitted",
        line.len(),
        t.elapsed().as_micros() as f64,
        emitted
    );
    println!(
        "bytes read total        : {} (file is {} bytes)",
        tail.bytes_read,
        std::fs::metadata(path).unwrap().len()
    );
}
