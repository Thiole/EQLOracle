//! Tests for `lib`. Kept out of the production module by
//! convention: src/ contains shipping code only.

use eqlp_core::{engine_from_toml, field, parse_buf, Outcome, ShapeMode};

const PACK: &str = r#"
name = "smoke"
version = 1

[[rule]]
id = "melee.hit"
kind = "damage"
anchors = ["points of damage"]
pattern = '^(?P<src>.+?) (?:hit|hits|slash|slashes|crush|crushes) (?P<dst>.+?) for (?P<amt>[0-9,]+) points? of damage\.'
examples = ["[Wed Aug 06 21:14:33 2025] You hit a decaying skeleton for 12 points of damage."]
[rule.fields]
amount = { from = "amt", as = "u64" }

[[rule]]
id = "death"
kind = "death"
anchors = ["have been slain by"]
pattern = '^You have been slain by (?P<killer>.+)!'
"#;

#[test]
fn end_to_end() {
    let eng = engine_from_toml(&[PACK]).unwrap();
    let log = b"[Wed Aug 06 21:14:33 2025] You hit a decaying skeleton for 12 points of damage.\n\
                [Wed Aug 06 21:14:34 2025] You have been slain by a decaying skeleton!\n\
                [Wed Aug 06 21:14:35 2025] You gain experience!!\n\
                not a log line at all\n\
                \n";
    let mut kinds = Vec::new();
    let cov = parse_buf(&eng, log, ShapeMode::DigitsAndNames, |line, o| {
        kinds.push(o.kind_str());
        if let Outcome::Matched(m) = o {
            if eng.rule(m.rule).id == "melee.hit" {
                let v = field::field(&eng, m, line, "amount");
                assert_eq!(v, field::Value::U64(12));
                assert_eq!(m.cap(line, 1).unwrap(), b"a decaying skeleton");
            }
        }
    });
    assert_eq!(
        kinds,
        ["matched", "matched", "unmatched", "headerless", "blank"]
    );
    assert_eq!(cov.total, 5);
    assert_eq!(cov.matched, 2);
    assert_eq!(cov.unmatched, 1);
    assert_eq!(cov.headerless, 1);
    assert_eq!(cov.blank, 1);
    assert!((cov.rate() - 2.0 / 3.0).abs() < 1e-9);
    assert_eq!(cov.distinct_shapes(), 1);
}

#[test]
fn later_pack_overrides_and_disables() {
    let over = r#"
name = "over"
version = 1
[[rule]]
id = "death"
enabled = false
pattern = "unused"
"#;
    let eng = engine_from_toml(&[PACK, over]).unwrap();
    assert_eq!(eng.rules().len(), 1);
    assert!(eng.find_rule("death").is_none());
}

#[test]
fn priority_is_deterministic() {
    let p = r#"
name = "p"
[[rule]]
id = "general"
priority = 0
anchors = ["points of damage"]
pattern = '^.+ for [0-9]+ points of damage\.'
[[rule]]
id = "specific"
priority = 10
anchors = ["points of damage"]
pattern = '^You .+ for [0-9]+ points of damage\.'
"#;
    let eng = engine_from_toml(&[p]).unwrap();
    let mut m = eng.matcher();
    let line = b"[Wed Aug 06 21:14:33 2025] You hit a rat for 3 points of damage.";
    match m.classify(line) {
        Outcome::Matched(mm) => assert_eq!(eng.rule(mm.rule).id, "specific"),
        o => panic!("{:?}", o),
    }
}
