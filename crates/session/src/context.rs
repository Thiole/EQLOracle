//! Context that encounters sit inside: zone, session, and anything later.
//!
//! Design notes: `docs/design/context.md`

use crate::timeline::Millis;

/// A labelled interval on the timeline. One `Spans` per dimension.
///
/// Zone and session need no columns on the encounter and no new tables. Both
/// are the same query as `state_at`: what was true at this instant. Storing a
/// `zone` field on each encounter would be a second copy that drifts the moment
/// a zone line arrives late or a pack is re-derived.
///
/// The same type serves any future dimension — raid target, group composition,
/// invocation, time of day — without a schema change.
#[derive(Debug, Clone, Default)]
pub struct Spans {
    starts: Vec<Millis>,
    labels: Vec<String>,
}

impl Spans {
    /// Mark that `label` became current at `ts`. Out-of-order insertion is
    /// handled, so a late line cannot corrupt lookups.
    pub fn enter(&mut self, ts: Millis, label: impl Into<String>) {
        let label = label.into();
        let at = self.starts.partition_point(|&x| x <= ts);
        // Collapse a repeat of the same label: re-entering a zone you are
        // already in is not a new span, and treating it as one would fragment
        // every grouping built on top.
        if at > 0 && self.labels[at - 1] == label {
            return;
        }
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

    /// Index of the span covering `ts`. Useful as a grouping key when the label
    /// itself repeats — you visit Nektulos Forest many times, and those are
    /// different visits.
    pub fn index_at(&self, ts: Millis) -> Option<usize> {
        let i = self.starts.partition_point(|&x| x <= ts);
        if i == 0 {
            None
        } else {
            Some(i - 1)
        }
    }

    /// The label of the span immediately before the one covering `ts` --
    /// what was current just before the most recent transition. Used to
    /// guess where on a freshly-entered zone's map the player probably is
    /// before any `/loc` has been typed there (the Maps module's entrance
    /// guess: match this against a `to_<zone>` marker). `None` if `ts`
    /// falls in the first span, or before the first mark entirely.
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

/// Derives session boundaries from silence in the log.
///
/// A session is a stretch of play. The log states no such thing, so it is
/// inferred from gaps — and because it is inferred, the threshold is a
/// parameter rather than a constant. On the reference log: 25 sessions at 5
/// minutes, 21 at 10, 16 at 60.
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

/// Everything an encounter sits inside. Adding a dimension is adding a field
/// here, not migrating stored data.
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

    /// Group encounter ids by the zone visit they started in.
    ///
    /// Keyed on the span index, not the zone name: you visit Nektulos Forest 35
    /// times and those are separate visits, not one bucket.
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
