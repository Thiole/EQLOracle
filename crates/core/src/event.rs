//! Values produced by the parse stage. Nothing here owns a `String`; a `Match`
//! is byte offsets into the caller's line buffer.
//!
//! Design notes: `docs/design/parsing.md`

/// Byte range into the line that produced it. `u32` because a log line longer
/// than 4 GiB is not a line.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    #[inline]
    pub fn new(start: usize, end: usize) -> Self {
        Span {
            start: start as u32,
            end: end as u32,
        }
    }
    #[inline]
    pub fn len(&self) -> usize {
        (self.end - self.start) as usize
    }
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.end == self.start
    }
    #[inline]
    pub fn slice<'a>(&self, line: &'a [u8]) -> &'a [u8] {
        &line[self.start as usize..self.end as usize]
    }
}

/// Wall-clock seconds as written in the log. No timezone is attached; see
/// `docs/design/parsing.md`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default)]
pub struct LocalTs(pub i64);

impl LocalTs {
    #[inline]
    pub fn secs(self) -> i64 {
        self.0
    }
}

/// Max capture groups carried from a rule. Exceeding it is a pack-compile
/// error, never a silent truncation.
pub const MAX_CAPS: usize = 12;

pub type RuleIdx = u32;

/// A line that matched a rule.
#[derive(Clone, Copy, Debug)]
pub struct Match {
    pub rule: RuleIdx,
    pub ts: LocalTs,
    /// Message text after the timestamp header.
    pub body: Span,
    /// Capture groups 1..=n, indexed from 0. `None` = group did not participate.
    /// Populated only when the matcher's capture mask covers this rule.
    pub caps: [Option<Span>; MAX_CAPS],
    pub ncaps: u8,
    /// False when the rule matched but captures were skipped. Call
    /// `Matcher::extract` to fill them in on demand.
    pub caps_extracted: bool,
}

impl Match {
    #[inline]
    pub fn cap<'a>(&self, line: &'a [u8], i: usize) -> Option<&'a [u8]> {
        debug_assert!(
            self.caps_extracted,
            "cap() on a match whose captures were skipped; call Matcher::extract first"
        );
        self.caps.get(i).copied().flatten().map(|s| s.slice(line))
    }
}

/// What happened to one line. Every line lands in exactly one variant.
#[derive(Clone, Copy, Debug)]
pub enum Outcome {
    /// Header parsed, a rule claimed the body.
    Matched(Match),
    /// Header parsed, no rule claimed the body.
    Unmatched { ts: LocalTs, body: Span },
    /// No recognisable timestamp header.
    Headerless { body: Span },
    /// Empty or whitespace-only.
    Blank,
}

impl Outcome {
    pub fn kind_str(&self) -> &'static str {
        match self {
            Outcome::Matched(_) => "matched",
            Outcome::Unmatched { .. } => "unmatched",
            Outcome::Headerless { .. } => "headerless",
            Outcome::Blank => "blank",
        }
    }
}
