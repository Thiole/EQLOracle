//! why: is the flat rule list actually costing anything? Histograms how
//! many rules survive anchor selection per line, and which candidate wins.
//! input: path to a real log
//! run: cargo run -p eqlp-app --release --example dispatch_stats -- <log>
use eqlp_app::ingest::framed_lines;
use eqlp_app::parser::build_engine;
use eqlp_core::Outcome;
use std::time::Instant;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: dispatch_stats <log>");
    let raw = std::fs::read(&path).unwrap_or_else(|e| panic!("couldn't read {path}: {e}"));
    let lines = framed_lines(&raw);
    let engine = build_engine().expect("pack builds");
    let mut m = engine.matcher();

    let mut cand_hist = [0u64; 12];
    let mut cands_total = 0u64;
    let mut matched = 0u64;
    let mut unmatched = 0u64;
    let mut other = 0u64;
    let mut zero_cand_unmatched = 0u64;

    let t = Instant::now(); // clock-exempt: benchmark
    for line in &lines {
        let out = m.classify(line);
        let n = m.last_candidate_count();
        cands_total += n as u64;
        cand_hist[n.min(11)] += 1;
        match out {
            Outcome::Matched(_) => matched += 1,
            Outcome::Unmatched { .. } => {
                unmatched += 1;
                if n == 0 {
                    zero_cand_unmatched += 1;
                }
            }
            _ => other += 1,
        }
    }
    let el = t.elapsed();

    let n = lines.len() as f64;
    println!(
        "lines: {} in {:?}  ({:.2}M lines/s, single thread)",
        lines.len(),
        el,
        n / el.as_secs_f64() / 1e6
    );
    println!("matched {matched}  unmatched {unmatched}  blank/headerless {other}");
    println!(
        "mean candidate rules per line: {:.3}",
        cands_total as f64 / n
    );
    println!(
        "unmatched lines that ran ZERO regexes: {zero_cand_unmatched} ({:.1}% of unmatched)",
        100.0 * zero_cand_unmatched as f64 / unmatched.max(1) as f64
    );
    println!("candidate-count histogram:");
    for (k, c) in cand_hist.iter().enumerate() {
        if *c > 0 {
            println!(
                "  {}{:<3} {:>10}  {:>5.2}%",
                if k == 11 { ">=" } else { "  " },
                k,
                c,
                100.0 * *c as f64 / n
            );
        }
    }
}
