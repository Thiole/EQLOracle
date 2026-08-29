//! Context that encounters sit inside: zone, session, and anything later.
//!
//! Design notes: `docs/design/context.md`

use crate::timeline::Millis;

/// why: a labelled interval, queried not stored -- avoids a drifting copy
#[derive(Debug, Clone, Default)]
pub struct Spans {
    starts: Vec<Millis>,
    labels: Vec<String>,
}

impl Spans {
    /// why: handles out-of-order insertion, a late line can't corrupt lookups
    ///
    /// Every enter is a new span, INCLUDING a consecutive same-label one
    /// -- player's own spec: "when you zone out, even if you zone back
    /// in, it should be considered a new zone". The old consecutive-
    /// same-label dedupe was checked against the real reference log
    /// before removal: 113 consecutive same-zone re-enters, every one
    /// at least 10s apart (71 of them 10+ minutes -- relogs, camps,
    /// instance re-entries), zero duplicate-print pairs -- the game
    /// emits exactly one "You have entered" per real zoning, so
    /// collapsing was pure information loss (a re-entered instance is a
    /// genuinely new visit).
    pub fn enter(&mut self, ts: Millis, label: impl Into<String>) {
        let label = label.into();
        let at = self.starts.partition_point(|&x| x <= ts);
        self.starts.insert(at, ts);
        self.labels.insert(at, label);
    }

    /// What was current at `ts`, or `None` before the first mark.
    pub fn at(&self, ts: Millis) -> Option<&str> {
        let i = self.starts.partition_point(|&x| x <= ts);
        if i == 0 {
            None
        } else {
            Some(&self.labels[i - 1])
        }
    }

    /// why: grouping key when the label repeats -- each visit stays distinct
    pub fn index_at(&self, ts: Millis) -> Option<usize> {
        let i = self.starts.partition_point(|&x| x <= ts);
        if i == 0 {
            None
        } else {
            Some(i - 1)
        }
    }

    /// why: prior span's label, feeds the Maps module's entrance guess
    pub fn label_before(&self, ts: Millis) -> Option<&str> {
        let i = self.index_at(ts)?;
        i.checked_sub(1).map(|j| self.labels[j].as_str())
    }

    pub fn iter(&self) -> impl Iterator<Item = (Millis, &str)> {
        self.starts
            .iter()
            .copied()
            .zip(self.labels.iter().map(|s| s.as_str()))
    }

    pub fn len(&self) -> usize {
        self.starts.len()
    }
    pub fn is_empty(&self) -> bool {
        self.starts.is_empty()
    }

    /// Start and end of the span at `i`. The last span is open-ended.
    pub fn bounds(&self, i: usize) -> Option<(Millis, Option<Millis>)> {
        let s = *self.starts.get(i)?;
        Some((s, self.starts.get(i + 1).copied()))
    }
}

/// why: session boundaries inferred from silence, threshold stays a param
#[derive(Debug, Clone)]
pub struct Sessions {
    gap_ms: Millis,
    last_ms: Option<Millis>,
    spans: Spans,
    n: usize,
}

impl Default for Sessions {
    fn default() -> Self {
        Sessions::new(600_000)
    }
}

impl Sessions {
    pub fn new(gap_ms: Millis) -> Self {
        Sessions {
            gap_ms,
            last_ms: None,
            spans: Spans::default(),
            n: 0,
        }
    }

    /// Feed every timestamped line, in order.
    pub fn observe(&mut self, ts: Millis) {
        let new_session = match self.last_ms {
            None => true,
            Some(p) => ts - p > self.gap_ms,
        };
        if new_session {
            self.n += 1;
            self.spans.enter(ts, format!("session-{}", self.n));
        }
        self.last_ms = Some(ts);
    }

    pub fn spans(&self) -> &Spans {
        &self.spans
    }
    pub fn count(&self) -> usize {
        self.n
    }
    pub fn at(&self, ts: Millis) -> Option<&str> {
        self.spans.at(ts)
    }
}

/// why: adding a dimension is a new field here, never a data migration
#[derive(Debug, Default)]
pub struct Context {
    pub zone: Spans,
    pub sessions: Sessions,
}

impl Context {
    pub fn new(session_gap_ms: Millis) -> Self {
        Context {
            zone: Spans::default(),
            sessions: Sessions::new(session_gap_ms),
        }
    }

    /// why: keyed on span index not zone name -- each visit stays distinct
    pub fn group_by_zone_visit<T: Copy>(
        &self,
        items: &[(T, Millis)],
    ) -> Vec<(usize, String, Vec<T>)> {
        let mut out: Vec<(usize, String, Vec<T>)> = Vec::new();
        for &(id, ts) in items {
            let (i, name) = match self.zone.index_at(ts) {
                Some(i) => (i, self.zone.at(ts).unwrap_or("?").to_string()),
                None => (usize::MAX, "unknown".to_string()),
            };
            match out.iter_mut().find(|(j, _, _)| *j == i) {
                Some((_, _, v)) => v.push(id),
                None => out.push((i, name, vec![id])),
            }
        }
        out
    }

    /// Group by zone name across all visits.
    pub fn group_by_zone_name<T: Copy>(&self, items: &[(T, Millis)]) -> Vec<(String, Vec<T>)> {
        let mut out: Vec<(String, Vec<T>)> = Vec::new();
        for &(id, ts) in items {
            let name = self.zone.at(ts).unwrap_or("unknown").to_string();
            match out.iter_mut().find(|(n, _)| *n == name) {
                Some((_, v)) => v.push(id),
                None => out.push((name, vec![id])),
            }
        }
        out
    }

    pub fn group_by_session<T: Copy>(&self, items: &[(T, Millis)]) -> Vec<(String, Vec<T>)> {
        let mut out: Vec<(String, Vec<T>)> = Vec::new();
        for &(id, ts) in items {
            let name = self.sessions.at(ts).unwrap_or("unknown").to_string();
            match out.iter_mut().find(|(n, _)| *n == name) {
                Some((_, v)) => v.push(id),
                None => out.push((name, vec![id])),
            }
        }
        out
    }
}
