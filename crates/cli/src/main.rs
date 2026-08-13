//! Headless harness.
//!
//! Everything you can do to a log, you can do without opening the app. That is
//! what makes CI possible: `eqlp lint` and `eqlp coverage --min-rate` are the
//! two commands a pre-commit hook needs, and neither one boots a webview.
//!
//!   eqlp lint    --pack p.toml [--pack q.toml] [--against log.txt --min-rate 0.95]
//!   eqlp parse   --pack p.toml LOG [--jsonl] [--only KIND]
//!   eqlp coverage --pack p.toml LOG [--top N] [--min-rate F]
//!   eqlp shapes  LOG [--top N] [--all] [--mode digits|names]
//!   eqlp bench   --pack p.toml LOG [--iters N]

use eqlp_core::{
    coverage::Coverage,
    engine::Engine,
    event::Outcome,
    frame, field,
    rule::{Pack, ResolvedPack},
    shape::{Shaper, ShapeMode},
};
use std::collections::HashMap;
use std::io::{BufWriter, Write};
use std::process::ExitCode;
use std::time::Instant;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("{}", USAGE);
        return ExitCode::from(2);
    }
    let cmd = args[0].clone();
    let a = Args::parse(&args[1..]);
    let r = match cmd.as_str() {
        "lint" => cmd_lint(&a),
        "parse" => cmd_parse(&a),
        "coverage" => cmd_coverage(&a),
        "shapes" => cmd_shapes(&a),
        "bench" => cmd_bench(&a),
        "-h" | "--help" | "help" => {
            println!("{USAGE}");
            Ok(())
        }
        other => Err(format!("unknown command '{other}'\n{USAGE}")),
    };
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

const USAGE: &str = "\
eqlp <lint|parse|coverage|shapes|bench> [options] [LOG]

  --pack PATH        rule pack; repeat to layer (last wins per rule id)
  --against PATH     lint: also require coverage over this log
  --min-rate F       fail if matched/(matched+unmatched) < F
  --top N            how many shapes/rules to show (default 20)
  --mode digits|names|aggressive   shape aggressiveness (default names)
  --jsonl            parse: emit one JSON object per matched line
  --only KIND        parse: restrict to rules with this kind
  --all              shapes: cluster every line, not just unmatched
  --iters N          bench: repeat count (default 5)";

// ---------------------------------------------------------------- args

#[derive(Default)]
struct Args {
    packs: Vec<String>,
    positional: Vec<String>,
    against: Option<String>,
    min_rate: Option<f64>,
    top: usize,
    mode: ShapeMode,
    jsonl: bool,
    all: bool,
    only: Option<String>,
    iters: usize,
}

impl Args {
    fn parse(v: &[String]) -> Args {
        let mut a = Args { top: 20, iters: 5, ..Default::default() };
        let mut i = 0;
        while i < v.len() {
            match v[i].as_str() {
                "--pack" => {
                    i += 1;
                    if let Some(p) = v.get(i) {
                        a.packs.push(p.clone())
                    }
                }
                "--against" => {
                    i += 1;
                    a.against = v.get(i).cloned();
                }
                "--min-rate" => {
                    i += 1;
                    a.min_rate = v.get(i).and_then(|s| s.parse().ok());
                }
                "--top" => {
                    i += 1;
                    a.top = v.get(i).and_then(|s| s.parse().ok()).unwrap_or(20);
                }
                "--iters" => {
                    i += 1;
                    a.iters = v.get(i).and_then(|s| s.parse().ok()).unwrap_or(5);
                }
                "--only" => {
                    i += 1;
                    a.only = v.get(i).cloned();
                }
                "--mode" => {
                    i += 1;
                    a.mode = match v.get(i).map(String::as_str) {
                        Some("digits") => ShapeMode::Digits,
                        Some("names") => ShapeMode::DigitsAndNames,
                        _ => ShapeMode::Aggressive,
                    };
                }
                "--jsonl" => a.jsonl = true,
                "--all" => a.all = true,
                s => a.positional.push(s.to_string()),
            }
            i += 1;
        }
        a
    }

    fn log(&self) -> Result<Vec<u8>, String> {
        let p = self
            .positional
            .first()
            .ok_or_else(|| "no log file given".to_string())?;
        std::fs::read(p).map_err(|e| format!("{p}: {e}"))
    }

    fn engine(&self) -> Result<Engine, String> {
        if self.packs.is_empty() {
            return Err("at least one --pack is required".into());
        }
        let mut packs = Vec::new();
        for p in &self.packs {
            let s = std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}"))?;
            packs.push(Pack::from_toml(&s).map_err(|e| format!("{p}: {e}"))?);
        }
        let resolved = ResolvedPack::layer(packs).map_err(|e| e.to_string())?;
        Engine::build(&resolved).map_err(|e| e.to_string())
    }
}

// ---------------------------------------------------------------- lint

fn cmd_lint(a: &Args) -> Result<(), String> {
    let eng = a.engine()?;
    let mut m = eng.matcher();
    let mut errors = 0usize;
    let mut warnings = 0usize;

    println!("pack sources: {}", eng.sources.join(" + "));
    println!("header:       {}", eng.header_name());
    println!("rules:        {}", eng.rules().len());

    // 1. Every example must be claimed by its own rule, running the whole
    //    engine. Catches shadowing by a higher-priority rule immediately.
    let mut examples_run = 0usize;
    for (i, r) in eng.rules().iter().enumerate() {
        for ex in &r.def.examples {
            examples_run += 1;
            match m.classify(ex.as_bytes()) {
                Outcome::Matched(mm) if mm.rule as usize == i => {}
                Outcome::Matched(mm) => {
                    errors += 1;
                    println!(
                        "ERROR {}: example claimed by '{}' instead\n      {}",
                        r.id,
                        eng.rule(mm.rule).id,
                        ex
                    );
                }
                other => {
                    errors += 1;
                    println!("ERROR {}: example is {} \n      {}", r.id, other.kind_str(), ex);
                }
            }
        }
        for cx in &r.def.counterexamples {
            examples_run += 1;
            let mut all = Vec::new();
            m.classify_all(cx.as_bytes(), &mut all);
            if all.contains(&(i as u32)) {
                errors += 1;
                println!("ERROR {}: counterexample matched\n      {}", r.id, cx);
            }
        }
    }

    // 2. Anchors must actually be substrings of every declared example,
    //    otherwise the prefilter silently makes the rule unreachable.
    for r in eng.rules() {
        for anc in &r.def.anchors {
            for ex in &r.def.examples {
                if !ex.contains(anc.as_str()) {
                    errors += 1;
                    println!("ERROR {}: anchor {:?} absent from its own example", r.id, anc);
                }
            }
        }
        for x in &r.def.excludes {
            for ex in &r.def.examples {
                if ex.contains(x.as_str()) {
                    errors += 1;
                    println!("ERROR {}: exclude {:?} present in its own example", r.id, x);
                }
            }
        }
        if r.def.anchors.is_empty() {
            warnings += 1;
            println!("WARN  {}: no anchors; runs its regex on every line", r.id);
        }
        if r.def.examples.is_empty() {
            warnings += 1;
            println!("WARN  {}: no examples; nothing pins this rule", r.id);
        }
    }

    // 3. Ambiguity: two rules that both match the same example.
    for (i, r) in eng.rules().iter().enumerate() {
        for ex in &r.def.examples {
            let mut all = Vec::new();
            m.classify_all(ex.as_bytes(), &mut all);
            if all.len() > 1 {
                let others: Vec<&str> = all
                    .iter()
                    .filter(|&&x| x as usize != i)
                    .map(|&x| eng.rule(x).id.as_str())
                    .collect();
                warnings += 1;
                println!(
                    "WARN  {}: example also matches {:?} — priority is deciding, not the pattern",
                    r.id, others
                );
            }
        }
    }

    // 4. Optional: coverage gate against a real log.
    if let Some(p) = &a.against {
        let buf = std::fs::read(p).map_err(|e| format!("{p}: {e}"))?;
        let cov = run(&eng, &buf, a.mode, |_, _| {});
        println!("\nagainst {p}: {:.2}% of {} event lines", cov.rate() * 100.0, cov.matched + cov.unmatched);
        let cold = cov.cold_rules();
        if !cold.is_empty() {
            warnings += cold.len();
            let names: Vec<&str> = cold.iter().map(|&i| eng.rule(i).id.as_str()).collect();
            println!("WARN  rules that never fired: {names:?}");
        }
        if let Some(min) = a.min_rate {
            if cov.rate() < min {
                errors += 1;
                println!("ERROR coverage {:.4} below --min-rate {:.4}", cov.rate(), min);
            }
        }
    }

    println!("\n{examples_run} assertions, {errors} errors, {warnings} warnings");
    if errors > 0 {
        Err(format!("{errors} lint errors"))
    } else {
        Ok(())
    }
}

// ---------------------------------------------------------------- parse

fn cmd_parse(a: &Args) -> Result<(), String> {
    let eng = a.engine()?;
    let buf = a.log()?;
    let stdout = std::io::stdout();
    let mut out = BufWriter::with_capacity(1 << 20, stdout.lock());

    let cov = run_masked(&eng, &buf, a.mode, true, |line, o| {
        if let Outcome::Matched(m) = o {
            let r = eng.rule(m.rule);
            if let Some(k) = &a.only {
                if &r.kind != k {
                    return;
                }
            }
            if a.jsonl {
                let mut obj = serde_json::Map::new();
                obj.insert("ts".into(), m.ts.0.into());
                obj.insert("rule".into(), r.id.clone().into());
                obj.insert("kind".into(), r.kind.clone().into());
                for (name, _) in &r.def.fields {
                    let v = match field::field(&eng, m, line, name) {
                        field::Value::Str(s) => serde_json::Value::String(lossy(s)),
                        field::Value::U64(n) => n.into(),
                        field::Value::I64(n) => n.into(),
                        field::Value::F64(n) => n.into(),
                        field::Value::Bool(b) => b.into(),
                        field::Value::Missing => serde_json::Value::Null,
                    };
                    obj.insert(name.clone(), v);
                }
                let _ = writeln!(out, "{}", serde_json::Value::Object(obj));
            } else {
                let _ = writeln!(out, "{}\t{}", r.id, lossy(m.body.slice(line)));
            }
        }
    });
    let _ = out.flush();
    eprintln!(
        "{} lines: {} matched, {} unmatched, {} headerless, {} blank ({:.2}%)",
        cov.total, cov.matched, cov.unmatched, cov.headerless, cov.blank, cov.rate() * 100.0
    );
    Ok(())
}

// ---------------------------------------------------------------- coverage

fn cmd_coverage(a: &Args) -> Result<(), String> {
    let eng = a.engine()?;
    let buf = a.log()?;
    let cov = run(&eng, &buf, a.mode, |_, _| {});

    println!("lines        {}", cov.total);
    println!("  matched    {}", cov.matched);
    println!("  unmatched  {}", cov.unmatched);
    println!("  headerless {}", cov.headerless);
    println!("  blank      {}", cov.blank);
    println!("coverage     {:.3}%", cov.rate() * 100.0);
    if let (Some(a0), Some(b0)) = (cov.first_ts, cov.last_ts) {
        println!("span         {} s", b0 - a0);
    }

    println!("\ntop rules");
    let mut per: Vec<(usize, u64)> = cov.per_rule.iter().copied().enumerate().collect();
    per.sort_unstable_by(|x, y| y.1.cmp(&x.1));
    for (i, c) in per.iter().take(a.top).filter(|(_, c)| *c > 0) {
        println!("{c:>10}  {}", eng.rule(*i as u32).id);
    }

    println!("\ntop unmatched shapes ({} distinct)", cov.distinct_shapes());
    for (sh, st) in cov.top_shapes(a.top) {
        println!("{:>10}  {}", st.count, lossy(sh));
        println!("            e.g. {}", lossy(&st.example));
    }
    if cov.shapes_overflow > 0 {
        println!("({} lines beyond the shape cap)", cov.shapes_overflow);
    }

    if let Some(min) = a.min_rate {
        if cov.rate() < min {
            return Err(format!("coverage {:.4} below --min-rate {min:.4}", cov.rate()));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- shapes

/// Runs with no pack at all. This is how you bootstrap: point it at a log you
/// have never seen and it hands you the templates, ranked.
fn cmd_shapes(a: &Args) -> Result<(), String> {
    let buf = a.log()?;
    let hdr = eqlp_core::header::BracketCtime;
    use eqlp_core::header::HeaderParser;

    let mut counts: HashMap<Vec<u8>, (u64, Vec<u8>)> = HashMap::new();
    let mut scratch = Vec::new();
    let mut shaper = Shaper::new();
    let mut total = 0u64;
    let mut headerless = 0u64;

    for line in frame::lines(&buf) {
        if line.iter().all(|b| b.is_ascii_whitespace()) {
            continue;
        }
        total += 1;
        let body = match hdr.parse(line) {
            Some((_, off)) => &line[off..],
            None => {
                headerless += 1;
                if !a.all {
                    continue;
                }
                line
            }
        };
        shaper.shape_into(body, a.mode, &mut scratch);
        let e = counts.entry(scratch.clone()).or_insert((0, body.to_vec()));
        e.0 += 1;
    }

    let mut v: Vec<_> = counts.into_iter().collect();
    v.sort_unstable_by(|x, y| y.1 .0.cmp(&x.1 .0));
    println!("{total} lines, {headerless} headerless, {} distinct shapes\n", v.len());
    let shown: u64 = v.iter().take(a.top).map(|(_, (c, _))| *c).sum();
    for (sh, (c, ex)) in v.iter().take(a.top) {
        println!("{c:>10}  {}", lossy(sh));
        println!("            e.g. {}", lossy(ex));
    }
    println!("\ntop {} shapes cover {:.1}% of lines", a.top, 100.0 * shown as f64 / total.max(1) as f64);
    Ok(())
}

// ---------------------------------------------------------------- bench

fn cmd_bench(a: &Args) -> Result<(), String> {
    let eng = a.engine()?;
    let buf = a.log()?;
    let nlines = frame::lines(&buf).count();

    // warm
    let _ = run(&eng, &buf, a.mode, |_, _| {});

    // A/B the capture mask, since it is the single biggest cost lever.
    let mut best_mo = f64::MAX;
    for _ in 0..a.iters.max(1) {
        let mut m = eng.matcher();
        m.capture_none();
        let t0 = Instant::now(); // clock-exempt: benchmark measures wall time by definition
        let mut sink = 0u64;
        for line in frame::lines(&buf) {
            if let Outcome::Matched(mm) = m.classify(line) {
                sink = sink.wrapping_add(mm.rule as u64);
            }
        }
        std::hint::black_box(sink);
        best_mo = best_mo.min(t0.elapsed().as_secs_f64());
    }

    let mut best = f64::MAX;
    for _ in 0..a.iters.max(1) {
        let mut m = eng.matcher();
        let t0 = Instant::now(); // clock-exempt: benchmark measures wall time by definition
        let mut sink = 0u64;
        for line in frame::lines(&buf) {
            if let Outcome::Matched(mm) = m.classify(line) {
                sink = sink.wrapping_add(mm.rule as u64);
            }
        }
        std::hint::black_box(sink);
        let el = t0.elapsed().as_secs_f64();
        best = best.min(el);
    }

    let mib = buf.len() as f64 / (1024.0 * 1024.0);
    println!("{} rules, {nlines} lines, {:.2} MiB", eng.rules().len(), mib);
    println!("best of {}: {:.4} s", a.iters.max(1), best);
    println!("  {:.1} MiB/s", mib / best);
    println!("  {:.2} M lines/s", nlines as f64 / best / 1e6);
    println!("  {:.0} ns/line", best * 1e9 / nlines.max(1) as f64);
    println!("capture mask off:  {:.0} ns/line  [{:.1} MiB/s]  ({:.1}x)",
        best_mo * 1e9 / nlines.max(1) as f64, mib / best_mo, best / best_mo);
    Ok(())
}

// ---------------------------------------------------------------- shared

fn run(eng: &Engine, buf: &[u8], mode: ShapeMode, mut f: impl FnMut(&[u8], &Outcome)) -> Coverage {
    run_masked(eng, buf, mode, false, f_wrap(&mut f))
}

fn f_wrap<'a>(f: &'a mut impl FnMut(&[u8], &Outcome)) -> impl FnMut(&[u8], &Outcome) + 'a {
    move |l, o| f(l, o)
}

fn run_masked(
    eng: &Engine,
    buf: &[u8],
    mode: ShapeMode,
    caps: bool,
    mut f: impl FnMut(&[u8], &Outcome),
) -> Coverage {
    let mut m = eng.matcher();
    if !caps {
        m.capture_none();
    }
    let mut cov = Coverage::new(eng.rules().len(), mode);
    for line in frame::lines(buf) {
        let out = m.classify(line);
        cov.record(line, &out);
        f(line, &out);
    }
    cov
}

fn lossy(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}
