//! Rule compilation and line classification.
//!
//! Two stages: an Aho-Corasick pass over literal anchors selects candidate
//! rules, then those candidates' regexes run. Anchors are an optimisation and
//! never change results; `excludes` are literal vetoes and do.
//!
//! Design notes: `docs/design/parsing.md`

use crate::event::{Match, Outcome, RuleIdx, Span, MAX_CAPS};
use crate::header::{self, HeaderParser};
use crate::rule::{PackError, ResolvedPack, RuleDef};
use aho_corasick::{AhoCorasick, AhoCorasickKind, MatchKind};
use regex::bytes::{CaptureLocations, Regex};

pub struct CompiledRule {
    pub id: String,
    pub kind: String,
    pub priority: i32,
    pub re: Regex,
    pub anchor_count: u16,
    /// Capture group names by group index (index 0 = whole match).
    pub cap_names: Vec<Option<String>>,
    pub def: RuleDef,
}

pub struct Engine {
    header: Box<dyn HeaderParser>,
    ac: Option<AhoCorasick>,
    /// literal pattern index -> rules that require it
    anchor_rules: Vec<Vec<RuleIdx>>,
    /// literal pattern index -> rules vetoed by its presence
    exclude_rules: Vec<Vec<RuleIdx>>,
    /// rules with no anchors at all; always candidates
    unanchored: Vec<RuleIdx>,
    rules: Vec<CompiledRule>,
    /// rule indices sorted by (priority desc, declaration order asc)
    eval_rank: Vec<u32>,
    pub sources: Vec<String>,
}

impl Engine {
    pub fn build(pack: &ResolvedPack) -> Result<Engine, PackError> {
        let hdr =
            header::by_name(&pack.header).unwrap_or_else(|| Box::new(crate::header::BracketCtime));

        let mut rules: Vec<CompiledRule> = Vec::with_capacity(pack.rules.len());
        let mut anchor_literals: Vec<String> = Vec::new();
        let mut anchor_rules: Vec<Vec<RuleIdx>> = Vec::new();
        let mut exclude_rules: Vec<Vec<RuleIdx>> = Vec::new();
        let mut unanchored: Vec<RuleIdx> = Vec::new();
        let mut anchor_index: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();

        for (i, d) in pack.rules.iter().enumerate() {
            if d.pattern.trim().is_empty() {
                return Err(PackError::EmptyPattern { id: d.id.clone() });
            }
            let re = Regex::new(&d.pattern).map_err(|e| PackError::BadRegex {
                id: d.id.clone(),
                msg: e.to_string(),
            })?;
            let ncaps = re.captures_len().saturating_sub(1);
            if ncaps > MAX_CAPS {
                return Err(PackError::TooManyCaps {
                    id: d.id.clone(),
                    n: ncaps,
                    max: MAX_CAPS,
                });
            }
            let cap_names = re.capture_names().map(|o| o.map(str::to_string)).collect();

            let idx = i as RuleIdx;
            if d.anchors.is_empty() {
                unanchored.push(idx);
            } else {
                for a in &d.anchors {
                    if a.is_empty() {
                        return Err(PackError::AnchorNotLiteral {
                            id: d.id.clone(),
                            anchor: a.clone(),
                        });
                    }
                    let pos = intern(
                        a,
                        &mut anchor_index,
                        &mut anchor_literals,
                        &mut anchor_rules,
                        &mut exclude_rules,
                    );
                    anchor_rules[pos].push(idx);
                }
            }

            for x in &d.excludes {
                if x.is_empty() {
                    return Err(PackError::AnchorNotLiteral {
                        id: d.id.clone(),
                        anchor: x.clone(),
                    });
                }
                let pos = intern(
                    x,
                    &mut anchor_index,
                    &mut anchor_literals,
                    &mut anchor_rules,
                    &mut exclude_rules,
                );
                exclude_rules[pos].push(idx);
            }

            rules.push(CompiledRule {
                id: d.id.clone(),
                kind: d.kind.clone(),
                priority: d.priority,
                re,
                anchor_count: d.anchors.len() as u16,
                cap_names,
                def: d.clone(),
            });
        }

        let ac = if anchor_literals.is_empty() {
            None
        } else {
            Some(
                AhoCorasick::builder()
                    .match_kind(MatchKind::Standard)
                    .kind(Some(AhoCorasickKind::DFA))
                    .build(&anchor_literals)
                    .expect("anchor automaton"),
            )
        };

        let mut eval_rank: Vec<u32> = (0..rules.len() as u32).collect();
        eval_rank.sort_by(|&a, &b| {
            rules[b as usize]
                .priority
                .cmp(&rules[a as usize].priority)
                .then(a.cmp(&b))
        });

        Ok(Engine {
            header: hdr,
            ac,
            anchor_rules,
            exclude_rules,
            unanchored,
            rules,
            eval_rank,
            sources: pack.sources.clone(),
        })
    }

    pub fn rules(&self) -> &[CompiledRule] {
        &self.rules
    }
    pub fn rule(&self, i: RuleIdx) -> &CompiledRule {
        &self.rules[i as usize]
    }
    pub fn find_rule(&self, id: &str) -> Option<RuleIdx> {
        self.rules
            .iter()
            .position(|r| r.id == id)
            .map(|i| i as RuleIdx)
    }
    pub fn header_name(&self) -> &'static str {
        self.header.name()
    }

    /// One matcher per thread. Holds the mutable scratch that keeps the hot
    /// path allocation-free; `Engine` itself is immutable and shareable.
    pub fn matcher(&self) -> Matcher<'_> {
        let locs = self
            .rules
            .iter()
            .map(|r| r.re.capture_locations())
            .collect();
        Matcher {
            eng: self,
            hits: vec![0u16; self.rules.len()],
            touched: Vec::with_capacity(16),
            seen_epoch: vec![0u32; self.anchor_rules.len()],
            veto: vec![0u32; self.rules.len()],
            epoch: 0,
            locs,
            cands: Vec::with_capacity(8),
            capture: vec![true; self.rules.len()],
        }
    }
}

/// Intern a literal into the shared Aho-Corasick alphabet, keeping the anchor
/// and exclude side-tables the same length as the literal list.
fn intern(
    lit: &str,
    index: &mut std::collections::HashMap<String, usize>,
    literals: &mut Vec<String>,
    anchors: &mut Vec<Vec<RuleIdx>>,
    excludes: &mut Vec<Vec<RuleIdx>>,
) -> usize {
    if let Some(&p) = index.get(lit) {
        return p;
    }
    literals.push(lit.to_string());
    anchors.push(Vec::new());
    excludes.push(Vec::new());
    let p = literals.len() - 1;
    index.insert(lit.to_string(), p);
    p
}

#[inline]
fn collect(locs: &CaptureLocations, off: usize) -> ([Option<Span>; MAX_CAPS], usize) {
    let mut caps = [None; MAX_CAPS];
    let n = locs.len().saturating_sub(1).min(MAX_CAPS);
    for g in 0..n {
        if let Some((s, e)) = locs.get(g + 1) {
            caps[g] = Some(Span::new(off + s, off + e));
        }
    }
    (caps, n)
}

pub struct Matcher<'e> {
    eng: &'e Engine,
    hits: Vec<u16>,
    touched: Vec<RuleIdx>,
    seen_epoch: Vec<u32>,
    veto: Vec<u32>,
    epoch: u32,
    locs: Vec<CaptureLocations>,
    cands: Vec<RuleIdx>,
    capture: Vec<bool>,
}

impl<'e> Matcher<'e> {
    pub fn engine(&self) -> &'e Engine {
        self.eng
    }

    /// Classify one framed line. Never panics, never allocates, always returns.
    pub fn classify(&mut self, line: &[u8]) -> Outcome {
        if line.iter().all(|b| b.is_ascii_whitespace()) {
            return Outcome::Blank;
        }

        let (ts, off) = match self.eng.header.parse(line) {
            Some(x) => x,
            None => {
                return Outcome::Headerless {
                    body: Span::new(0, line.len()),
                }
            }
        };
        let body = &line[off..];
        let body_span = Span::new(off, line.len());

        self.select_candidates(body);

        for k in 0..self.cands.len() {
            let ri = self.cands[k];
            let rule = &self.eng.rules[ri as usize];

            if !self.capture[ri as usize] {
                // Boolean match only. ~3x cheaper than capture extraction, and
                // for a consumer that ignores this rule the captures are waste.
                if rule.re.is_match(body) {
                    return Outcome::Matched(Match {
                        rule: ri,
                        ts,
                        body: body_span,
                        caps: [None; MAX_CAPS],
                        ncaps: 0,
                        caps_extracted: false,
                    });
                }
                continue;
            }

            let locs = &mut self.locs[ri as usize];
            if rule.re.captures_read(locs, body).is_some() {
                let (caps, n) = collect(locs, off);
                return Outcome::Matched(Match {
                    rule: ri,
                    ts,
                    body: body_span,
                    caps,
                    ncaps: n as u8,
                    caps_extracted: true,
                });
            }
        }

        Outcome::Unmatched {
            ts,
            body: body_span,
        }
    }

    /// Which rules are worth running a regex for. Fills `self.cands` in
    /// evaluation order (priority desc, then declaration order).
    fn select_candidates(&mut self, body: &[u8]) {
        self.cands.clear();

        for &r in &self.eng.unanchored {
            self.hits[r as usize] = self.eng.rules[r as usize].anchor_count; // == 0
        }

        if let Some(ac) = &self.eng.ac {
            self.epoch = self.epoch.wrapping_add(1);
            if self.epoch == 0 {
                // Wrapped: clear rather than risk a stale epoch collision.
                self.seen_epoch.iter_mut().for_each(|e| *e = 0);
                self.veto.iter_mut().for_each(|e| *e = 0);
                self.epoch = 1;
            }
            for m in ac.find_overlapping_iter(body) {
                let pat = m.pattern().as_usize();
                if self.seen_epoch[pat] == self.epoch {
                    continue; // same anchor twice in one line
                }
                self.seen_epoch[pat] = self.epoch;
                for &r in &self.eng.anchor_rules[pat] {
                    let h = &mut self.hits[r as usize];
                    if *h == 0 {
                        self.touched.push(r);
                    }
                    *h += 1;
                }
                for &r in &self.eng.exclude_rules[pat] {
                    self.veto[r as usize] = self.epoch;
                }
            }
        }

        // A rule qualifies when every anchor it declared was present.
        for &r in &self.touched {
            if self.hits[r as usize] >= self.eng.rules[r as usize].anchor_count
                && self.veto[r as usize] != self.epoch
            {
                self.cands.push(r);
            }
            self.hits[r as usize] = 0;
        }
        self.touched.clear();

        for &r in &self.eng.unanchored {
            if self.veto[r as usize] != self.epoch {
                self.cands.push(r);
            }
        }

        if self.cands.len() > 1 {
            let eng = self.eng;
            self.cands.sort_unstable_by(|&a, &b| {
                eng.rules[b as usize]
                    .priority
                    .cmp(&eng.rules[a as usize].priority)
                    .then(a.cmp(&b))
            });
        }
    }

    /// Which rules get their capture groups pulled out. Everything else gets a
    /// boolean match, which is roughly 3x cheaper.
    ///
    /// This is a runtime knob rather than a pack setting on purpose: which
    /// fields matter depends on which consumers are attached right now. A DPS
    /// meter alone wants damage captures and nothing else; open a loot panel
    /// and the mask widens. Cost tracks what is actually being read.
    pub fn capture_only(&mut self, rules: &[RuleIdx]) {
        self.capture.iter_mut().for_each(|c| *c = false);
        for &r in rules {
            if let Some(c) = self.capture.get_mut(r as usize) {
                *c = true;
            }
        }
    }

    pub fn capture_all(&mut self) {
        self.capture.iter_mut().for_each(|c| *c = true);
    }

    pub fn capture_none(&mut self) {
        self.capture.iter_mut().for_each(|c| *c = false);
    }

    /// Fill in captures for a match that skipped them. Idempotent.
    pub fn extract(&mut self, line: &[u8], m: &mut Match) {
        if m.caps_extracted {
            return;
        }
        let off = m.body.start as usize;
        let body = &line[off..];
        let ri = m.rule as usize;
        let locs = &mut self.locs[ri];
        if self.eng.rules[ri].re.captures_read(locs, body).is_some() {
            let (caps, n) = collect(locs, off);
            m.caps = caps;
            m.ncaps = n as u8;
            m.caps_extracted = true;
        }
    }

    /// Every rule that matches, not just the winner. Used by `lint` to detect
    /// ambiguity, and by anyone who genuinely wants overlapping emission.
    pub fn classify_all(&mut self, line: &[u8], out: &mut Vec<RuleIdx>) {
        out.clear();
        let off = match self.eng.header.parse(line) {
            Some((_, o)) => o,
            None => return,
        };
        let body = &line[off..];
        self.select_candidates(body);
        for k in 0..self.cands.len() {
            let ri = self.cands[k];
            if self.eng.rules[ri as usize].re.is_match(body) {
                out.push(ri);
            }
        }
    }
}

/// Convenience for `eval_rank` consumers and tests.
impl Engine {
    pub fn eval_order(&self) -> &[u32] {
        &self.eval_rank
    }
}
