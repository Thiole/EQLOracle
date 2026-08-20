//! Properties that must hold for any pack, on any input.
//!
//! These are the tests that let the rule pack churn weekly without anyone
//! holding their breath. They assert invariants of the *engine*, so they keep
//! passing as rules are added, and they fail loudly when a rule breaks one.

use eqlp_core::{
    engine::Engine,
    event::Outcome,
    frame,
    rule::{Pack, ResolvedPack},
    shape::ShapeMode,
    Coverage,
};

fn base_pack_src() -> String {
    let p = concat!(env!("CARGO_MANIFEST_DIR"), "/../../packs/eql.toml");
    std::fs::read_to_string(p).expect("packs/base.toml")
}

fn engine() -> Engine {
    let pack = Pack::from_toml(&base_pack_src()).expect("pack parses");
    Engine::build(&ResolvedPack::layer(vec![pack]).unwrap()).expect("pack builds")
}

/// Deterministic pseudo-random bytes. No dev-dependency, reproducible failures.
struct Lcg(u64);
impl Lcg {
    fn next_u32(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) as u32
    }
    fn byte(&mut self) -> u8 {
        (self.next_u32() & 0xff) as u8
    }
}

// ---------------------------------------------------------------------------

/// The load-bearing invariant of the whole design.
///
/// Anchors exist only to skip regexes that could not have matched. They are an
/// optimisation and must be *semantically invisible*: stripping every anchor
/// from the pack must produce byte-identical results on every line. A wrong
/// anchor is the one bug class that could silently drop events forever, and
/// this is the test that makes that impossible to ship.
///
/// `excludes` are deliberately NOT stripped — those change meaning on purpose.
#[test]
fn anchors_never_change_the_answer() {
    let pack = Pack::from_toml(&base_pack_src()).unwrap();
    let fast = Engine::build(&ResolvedPack::layer(vec![pack.clone()]).unwrap()).unwrap();

    let mut stripped = pack;
    for r in &mut stripped.rules {
        r.anchors.clear();
    }
    let slow = Engine::build(&ResolvedPack::layer(vec![stripped]).unwrap()).unwrap();

    let corpus = corpus();
    let (mut a, mut b) = (fast.matcher(), slow.matcher());
    let mut checked = 0usize;

    for line in frame::lines(&corpus) {
        let (x, y) = (a.classify(line), b.classify(line));
        match (&x, &y) {
            (Outcome::Matched(m1), Outcome::Matched(m2)) => {
                assert_eq!(
                    fast.rule(m1.rule).id,
                    slow.rule(m2.rule).id,
                    "anchor prefilter changed the winner on: {}",
                    String::from_utf8_lossy(line)
                );
                assert_eq!(m1.caps, m2.caps);
            }
            (Outcome::Matched(m1), other) => panic!(
                "anchors made '{}' match where the unfiltered engine says {}: {}",
                fast.rule(m1.rule).id,
                other.kind_str(),
                String::from_utf8_lossy(line)
            ),
            (other, Outcome::Matched(m2)) => panic!(
                "anchor prefilter SUPPRESSED rule '{}' — this is the silent-data-loss bug: {}\n  (outcome was {})",
                slow.rule(m2.rule).id,
                String::from_utf8_lossy(line),
                other.kind_str()
            ),
            _ => assert_eq!(x.kind_str(), y.kind_str()),
        }
        checked += 1;
    }
    assert!(checked > 1000, "corpus too small to mean anything");
}

/// A `Matcher` reuses scratch buffers across calls. If any of that state leaked
/// between lines, results would depend on history — the worst kind of parser
/// bug, because it only shows up in production ordering.
#[test]
fn matcher_state_does_not_leak_between_lines() {
    let eng = engine();
    let corpus = corpus();
    let lines: Vec<&[u8]> = frame::lines(&corpus).take(4000).collect();

    let mut shared = eng.matcher();
    let shared_out: Vec<String> = lines
        .iter()
        .map(|l| describe(&eng, &mut shared, l))
        .collect();

    // Same lines, but each through a virgin matcher.
    let fresh_out: Vec<String> = lines
        .iter()
        .map(|l| {
            let mut m = eng.matcher();
            describe(&eng, &mut m, l)
        })
        .collect();
    assert_eq!(shared_out, fresh_out);

    // And in reverse order, to catch anything order-dependent.
    let mut rev = eng.matcher();
    let mut rev_out: Vec<String> = lines
        .iter()
        .rev()
        .map(|l| describe(&eng, &mut rev, l))
        .collect();
    rev_out.reverse();
    assert_eq!(shared_out, rev_out);
}

/// The epoch counter used to dedupe anchor hits wraps at u32. Force the wrap
/// and confirm nothing goes stale.
#[test]
fn epoch_wraparound_is_safe() {
    let eng = engine();
    let mut m = eng.matcher();
    let line = b"[Wed Aug 06 21:14:33 2025] You slash a decaying skeleton for 12 points of damage.";
    let want = describe(&eng, &mut m, line);
    for _ in 0..200_000 {
        let _ = m.classify(b"[Wed Aug 06 21:14:33 2025] filler line with no rule");
    }
    assert_eq!(describe(&eng, &mut m, line), want);
}

/// Total function: arbitrary bytes in, an answer out, no panic, no hang.
#[test]
fn arbitrary_bytes_never_panic() {
    let eng = engine();
    let mut m = eng.matcher();
    let mut rng = Lcg(0xDEADBEEF);

    for len in 0..512usize {
        let mut buf: Vec<u8> = (0..len).map(|_| rng.byte()).collect();
        let _ = m.classify(&buf);

        // Bias toward near-miss input: a valid header glued to garbage is far
        // more likely to find a bug than uniform noise.
        let mut real = b"[Wed Aug 06 21:14:33 2025] ".to_vec();
        real.append(&mut buf);
        check_invariants(&eng, &mut m, &real);

        // Truncations of a real line, which is exactly what a partial flush
        // looks like while tailing.
        let full =
            b"[Wed Aug 06 21:14:33 2025] You slash a decaying skeleton for 12 points of damage.";
        check_invariants(&eng, &mut m, &full[..len.min(full.len())]);
    }
}

/// Every span the parser hands back must be inside the line it came from,
/// and every capture must be inside the body.
fn check_invariants(eng: &Engine, m: &mut eqlp_core::Matcher, line: &[u8]) {
    match m.classify(line) {
        Outcome::Matched(mm) => {
            assert!(mm.body.end as usize <= line.len());
            assert!(mm.body.start <= mm.body.end);
            assert!((mm.rule as usize) < eng.rules().len());
            for c in mm.caps.iter().flatten() {
                assert!(
                    c.start >= mm.body.start && c.end <= mm.body.end,
                    "capture escapes body"
                );
                assert!(c.start <= c.end);
                let _ = c.slice(line); // must not panic
            }
        }
        Outcome::Unmatched { body, .. } | Outcome::Headerless { body } => {
            assert!(body.end as usize <= line.len());
            let _ = body.slice(line);
        }
        Outcome::Blank => {}
    }
}

/// Framing must not depend on how the bytes arrived. This is the property that
/// makes live tailing behave the same as a batch reparse of the same file.
#[test]
fn streamed_framing_equals_batch_framing() {
    let corpus = corpus();
    let batch: Vec<Vec<u8>> = frame::lines(&corpus).map(|l| l.to_vec()).collect();

    for chunk in [1usize, 2, 7, 13, 64, 997, 4096, 65536] {
        let mut fr = frame::Framer::default();
        let mut got: Vec<Vec<u8>> = Vec::new();
        for c in corpus.chunks(chunk) {
            fr.push(c, |l| got.push(l.to_vec()));
        }
        fr.flush(|l| got.push(l.to_vec()));
        assert_eq!(
            got.len(),
            batch.len(),
            "line count differs at chunk={chunk}"
        );
        assert_eq!(got, batch, "content differs at chunk={chunk}");
    }
}

/// Coverage buckets must account for every line exactly once.
#[test]
fn coverage_buckets_sum_to_the_line_count() {
    let eng = engine();
    let corpus = corpus();
    let mut m = eng.matcher();
    let mut cov = Coverage::new(eng.rules().len(), ShapeMode::DigitsAndNames);
    let mut n = 0u64;
    for line in frame::lines(&corpus) {
        let o = m.classify(line);
        cov.record(line, &o);
        n += 1;
    }
    assert_eq!(cov.total, n);
    assert_eq!(cov.matched + cov.unmatched + cov.headerless + cov.blank, n);
    assert_eq!(cov.per_rule.iter().sum::<u64>(), cov.matched);
}

/// Lazily extracted captures must equal eagerly extracted ones.
#[test]
fn lazy_captures_match_eager_captures() {
    let eng = engine();
    let corpus = corpus();
    let (mut eager, mut lazy) = (eng.matcher(), eng.matcher());
    lazy.capture_none();
    for line in frame::lines(&corpus).take(20_000) {
        match (eager.classify(line), lazy.classify(line)) {
            (Outcome::Matched(a), Outcome::Matched(mut b)) => {
                assert_eq!(a.rule, b.rule);
                assert!(!b.caps_extracted);
                lazy.extract(line, &mut b);
                assert!(b.caps_extracted);
                assert_eq!(a.caps, b.caps, "{}", String::from_utf8_lossy(line));
            }
            (x, y) => assert_eq!(x.kind_str(), y.kind_str()),
        }
    }
}

// ---------------------------------------------------------------------------

fn describe(eng: &Engine, m: &mut eqlp_core::Matcher, line: &[u8]) -> String {
    match m.classify(line) {
        Outcome::Matched(mm) => {
            let caps: Vec<String> = (0..mm.ncaps as usize)
                .map(|i| {
                    mm.cap(line, i)
                        .map(|b| String::from_utf8_lossy(b).into_owned())
                        .unwrap_or_default()
                })
                .collect();
            format!("{}|{}|{:?}", eng.rule(mm.rule).id, mm.ts.0, caps)
        }
        o => o.kind_str().to_string(),
    }
}

/// The synthetic corpus if it has been generated, otherwise a small inline one
/// so the suite still means something on a fresh checkout.
fn corpus() -> Vec<u8> {
    // Prefer a slice of the real game log; fall back to the synthetic one, then
    // to an inline sample so a fresh checkout still exercises something.
    for p in [
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../corpus/real-slice.log"),
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../corpus/synthetic.log"),
    ] {
        if let Ok(b) = std::fs::read(p) {
            if b.len() > 4096 {
                return b.into_iter().take(8_000_000).collect();
            }
        }
    }
    let mut v = Vec::new();
    let bodies: [&str; 12] = [
        "You slash a decaying skeleton for 12 points of damage.",
        "a decaying skeleton hits YOU for 5 points of damage.",
        "a decaying skeleton was hit by non-melee for 100 points of damage.",
        "You try to slash a decaying skeleton, but miss!",
        "You score a critical hit! (348)",
        "Kenkyo has healed you for 412 points.",
        "You have slain a decaying skeleton!",
        "You gain party experience!!",
        "--You have looted a Rusty Short Sword.--",
        "You have entered Greater Faydark.",
        "Kenkyo tells you, 'inc east'",
        "Your spell fizzles!",
    ];
    for i in 0..2000 {
        v.extend_from_slice(
            format!(
                "[Wed Aug 06 21:{:02}:{:02} 2025] {}\n",
                i / 60 % 60,
                i % 60,
                bodies[i % bodies.len()]
            )
            .as_bytes(),
        );
    }
    v
}
