//! Reconstructing what was true at an arbitrary instant.
//!
//! Design notes: `docs/design/timeline.md`

use std::collections::HashMap;

pub type Millis = i64;

/// What an entity was doing. Ordered by how much it stops them acting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum State {
    /// Fighting, or able to.
    #[default]
    Engaged,
    /// Mesmerized. Still in the fight and still on the aggro list — this delays
    /// actions, it does not end combat. A mezzed mob must not close an
    /// encounter and must not count as removed from the field.
    Mezzed,
    /// Charmed. Changes side; damage it deals now counts for the charmer's
    /// team, though the log names no owner.
    Charmed,
    Dead,
    /// Left the fight for a reason the log does not report: memory blur,
    /// pacify, lull, fleeing, out of range. Inferred from silence, never
    /// observed, and named so that is obvious.
    Lost,
}

impl State {
    /// Whether this state keeps an encounter alive. Mezzed does — the mob is
    /// still there and will wake up.
    pub fn in_combat(self) -> bool {
        matches!(self, State::Engaged | State::Mezzed | State::Charmed)
    }

    pub fn name(self) -> &'static str {
        match self {
            State::Engaged => "engaged",
            State::Mezzed => "mezzed",
            State::Charmed => "charmed",
            State::Dead => "dead",
            State::Lost => "lost",
        }
    }
}

/// Why a transition happened. Kept because an inferred transition is worth
/// less than an observed one and the UI should be able to say so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cause {
    /// A log line stated it.
    Observed,
    /// Derived from the absence of evidence.
    Inferred,
}

#[derive(Debug, Clone, Copy)]
pub struct Transition {
    pub ts: Millis,
    pub entity: u32,
    pub state: State,
    pub cause: Cause,
}

/// Append-only transition log, queryable at any instant.
///
/// Deliberately not a `HashMap<Entity, State>`. Current state is a special case
/// of state-at-time, and keeping a mutable copy would be a second source of
/// truth that drifts from the timeline the moment anything is replayed or
/// re-derived.
#[derive(Debug, Default)]
pub struct Timeline {
    /// Per entity, transitions in ascending timestamp order.
    by_entity: HashMap<u32, Vec<Transition>>,
    all: Vec<Transition>,
}

impl Timeline {
    /// Record a transition. Out-of-order arrivals are inserted in position, so
    /// a late line cannot corrupt the ordering queries depend on.
    pub fn push(&mut self, t: Transition) {
        let v = self.by_entity.entry(t.entity).or_default();
        let at = v.partition_point(|x| x.ts <= t.ts);
        v.insert(at, t);
        let at = self.all.partition_point(|x| x.ts <= t.ts);
        self.all.insert(at, t);
    }

    pub fn observed(&mut self, ts: Millis, entity: u32, state: State) {
        self.push(Transition {
            ts,
            entity,
            state,
            cause: Cause::Observed,
        });
    }

    pub fn inferred(&mut self, ts: Millis, entity: u32, state: State) {
        self.push(Transition {
            ts,
            entity,
            state,
            cause: Cause::Inferred,
        });
    }

    /// State of `entity` at `ts` — the last transition at or before it.
    ///
    /// This is the scrub primitive: drag to any instant and every entity's
    /// state falls out of the same timeline that produced the damage numbers.
    pub fn state_at(&self, entity: u32, ts: Millis) -> Option<(State, Cause)> {
        let v = self.by_entity.get(&entity)?;
        let i = v.partition_point(|x| x.ts <= ts);
        if i == 0 {
            None
        } else {
            Some((v[i - 1].state, v[i - 1].cause))
        }
    }

    /// State of every listed entity at `ts`. Entities with no transition yet
    /// are reported `Engaged`: they were seen fighting, nothing has changed.
    pub fn snapshot(&self, entities: &[u32], ts: Millis) -> Vec<(u32, State, Cause)> {
        entities
            .iter()
            .map(|&e| {
                let (s, c) = self
                    .state_at(e, ts)
                    .unwrap_or((State::Engaged, Cause::Inferred));
                (e, s, c)
            })
            .collect()
    }

    /// Whether any entity is still in combat at `ts`. Mezzed counts.
    pub fn any_in_combat(&self, entities: &[u32], ts: Millis) -> bool {
        self.snapshot(entities, ts)
            .iter()
            .any(|(_, s, _)| s.in_combat())
    }

    /// Transitions inside a window, for drawing markers on a scrub bar.
    pub fn between(&self, from: Millis, to: Millis) -> &[Transition] {
        let a = self.all.partition_point(|x| x.ts < from);
        let b = self.all.partition_point(|x| x.ts <= to);
        &self.all[a..b]
    }

    pub fn transitions_of(&self, entity: u32) -> &[Transition] {
        self.by_entity
            .get(&entity)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    pub fn len(&self) -> usize {
        self.all.len()
    }
    pub fn is_empty(&self) -> bool {
        self.all.is_empty()
    }
}

/// A damage sample bucketed for plotting.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bucket {
    pub start_ms: Millis,
    pub total: u64,
    pub events: u32,
}

impl Bucket {
    pub fn dps(&self, width_ms: Millis) -> f64 {
        if width_ms <= 0 {
            0.0
        } else {
            self.total as f64 / (width_ms as f64 / 1000.0)
        }
    }
}

/// Bucket damage into a fixed-width series for a graph.
///
/// `ts` and `amount` must be the same length and `ts` ascending. Empty buckets
/// are emitted rather than skipped: a gap in a fight is information, and a
/// series with holes cannot be plotted against a linear axis.
pub fn series(
    ts: &[Millis],
    amount: &[u64],
    from: Millis,
    to: Millis,
    width_ms: Millis,
) -> Vec<Bucket> {
    let width = width_ms.max(1);
    if to < from {
        return Vec::new();
    }
    let n = ((to - from) / width + 1) as usize;
    let mut out: Vec<Bucket> = (0..n)
        .map(|i| Bucket {
            start_ms: from + i as i64 * width,
            total: 0,
            events: 0,
        })
        .collect();
    for (&t, &a) in ts.iter().zip(amount) {
        if t < from || t > to {
            continue;
        }
        let i = ((t - from) / width) as usize;
        if let Some(b) = out.get_mut(i) {
            b.total += a;
            b.events += 1;
        }
    }
    out
}
