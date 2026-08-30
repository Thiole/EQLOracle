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
    /// why: still on the aggro list -- delays action, never closes the fight
    Mezzed,
    /// why: switches side, log never names the charmer
    Charmed,
    Dead,
    /// why: left for an unreported reason, inferred never observed
    Lost,
}

impl State {
    /// why: Mezzed keeps a fight alive -- the mob is still there
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

/// why: an inferred transition is worth less, UI can say so
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

/// why: append-only, no mutable-copy shortcut that could drift on replay
#[derive(Debug, Default)]
pub struct Timeline {
    /// Per entity, transitions in ascending timestamp order.
    by_entity: HashMap<u32, Vec<Transition>>,
    all: Vec<Transition>,
}

impl Timeline {
    /// why: inserts in position, a late line can't corrupt ordering
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

    /// why: the scrub primitive -- last transition at or before `ts`
    pub fn state_at(&self, entity: u32, ts: Millis) -> Option<(State, Cause)> {
        self.state_since(entity, ts).map(|(s, c, _)| (s, c))
    }

    /// why: state plus when it began -- drop_watch's loot grace needs "dead how long"
    pub fn state_since(&self, entity: u32, ts: Millis) -> Option<(State, Cause, Millis)> {
        let v = self.by_entity.get(&entity)?;
        let i = v.partition_point(|x| x.ts <= ts);
        if i == 0 {
            None
        } else {
            Some((v[i - 1].state, v[i - 1].cause, v[i - 1].ts))
        }
    }

    /// why: no-transition entities default to Engaged -- nothing changed yet
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

/// why: empty buckets emitted not skipped -- a gap is real information
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
