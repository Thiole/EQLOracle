//! Per-line accounting and unmatched-shape clustering.
//!
//! Design notes: `docs/design/parsing.md`

use crate::event::{Outcome, RuleIdx};
use crate::shape::{ShapeMode, Shaper};
use std::collections::HashMap;

pub const DEFAULT_SHAPE_CAP: usize = 4096;

#[derive(Debug, Clone)]
pub struct ShapeStat {
    pub count: u64,
    pub example: Vec<u8>,
}

pub struct Coverage {
    pub total: u64,
    pub matched: u64,
    pub unmatched: u64,
    pub headerless: u64,
    pub blank: u64,
    pub per_rule: Vec<u64>,
    pub first_ts: Option<i64>,
    pub last_ts: Option<i64>,
    shapes: HashMap<Vec<u8>, ShapeStat>,
    shape_cap: usize,
    pub shapes_overflow: u64,
    mode: ShapeMode,
    scratch: Vec<u8>,
    shaper: Shaper,
}

impl Coverage {
    pub fn new(nrules: usize, mode: ShapeMode) -> Self {
        Coverage {
            total: 0,
            matched: 0,
            unmatched: 0,
            headerless: 0,
            blank: 0,
            per_rule: vec![0; nrules],
            first_ts: None,
            last_ts: None,
            shapes: HashMap::new(),
            shape_cap: DEFAULT_SHAPE_CAP,
            shapes_overflow: 0,
            mode,
            scratch: Vec::with_capacity(256),
            shaper: Shaper::new(),
        }
    }

    pub fn with_shape_cap(mut self, cap: usize) -> Self {
        self.shape_cap = cap;
        self
    }

    pub fn record(&mut self, line: &[u8], out: &Outcome) {
        self.total += 1;
        match out {
            Outcome::Matched(m) => {
                self.matched += 1;
                if let Some(c) = self.per_rule.get_mut(m.rule as usize) {
                    *c += 1;
                }
                self.note_ts(m.ts.0);
            }
            Outcome::Unmatched { ts, body } => {
                self.unmatched += 1;
                self.note_ts(ts.0);
                let b = body.slice(line);
                self.shaper.shape_into(b, self.mode, &mut self.scratch);
                if let Some(s) = self.shapes.get_mut(&self.scratch) {
                    s.count += 1;
                } else if self.shapes.len() < self.shape_cap {
                    self.shapes.insert(
                        self.scratch.clone(),
                        ShapeStat {
                            count: 1,
                            example: b.to_vec(),
                        },
                    );
                } else {
                    self.shapes_overflow += 1;
                }
            }
            Outcome::Headerless { .. } => self.headerless += 1,
            Outcome::Blank => self.blank += 1,
        }
    }

    #[inline]
    fn note_ts(&mut self, t: i64) {
        if self.first_ts.is_none() {
            self.first_ts = Some(t);
        }
        self.last_ts = Some(t);
    }

    /// Fraction of timestamped, non-blank lines that a rule claimed. Headerless
    /// and blank lines are excluded from the denominator because they are not
    /// events and counting them just flatters the number.
    pub fn rate(&self) -> f64 {
        let d = self.matched + self.unmatched;
        if d == 0 {
            1.0
        } else {
            self.matched as f64 / d as f64
        }
    }

    pub fn top_shapes(&self, n: usize) -> Vec<(&[u8], &ShapeStat)> {
        let mut v: Vec<_> = self.shapes.iter().map(|(k, s)| (k.as_slice(), s)).collect();
        v.sort_unstable_by(|a, b| b.1.count.cmp(&a.1.count).then(a.0.cmp(b.0)));
        v.truncate(n);
        v
    }

    pub fn distinct_shapes(&self) -> usize {
        self.shapes.len()
    }

    /// Rules that never fired. Either dead weight or a regression — either way
    /// you want to know, and `lint --against <log>` fails on it in CI.
    pub fn cold_rules(&self) -> Vec<RuleIdx> {
        self.per_rule
            .iter()
            .enumerate()
            .filter(|(_, &c)| c == 0)
            .map(|(i, _)| i as RuleIdx)
            .collect()
    }
}
