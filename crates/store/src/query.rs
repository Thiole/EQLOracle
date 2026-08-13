//! Aggregation over the store. Every number the UI shows is one of these.
//!
//! Design notes: `docs/design/store.md`

use crate::ability::{AbilityId, Sym, Tags};
use crate::store::{EncounterId, EventKind, Flags, Store};
use eqlp_source::Millis;
use std::collections::HashMap;

/// What to include. An empty field means "no constraint".
#[derive(Debug, Clone, Default)]
pub struct Filter {
    pub encounter: Option<EncounterId>,
    pub actor: Option<Sym>,
    pub target: Option<Sym>,
    pub kind: Option<EventKind>,
    /// Ability must carry all of these tags.
    pub tags_all: Tags,
    /// Ability must carry at least one of these. Zero means no constraint.
    pub tags_any: Tags,
    pub since_ms: Option<Millis>,
    pub until_ms: Option<Millis>,
}

impl Filter {
    pub fn encounter(id: EncounterId) -> Self {
        Filter { encounter: Some(id), ..Default::default() }
    }
    pub fn damage(mut self) -> Self {
        self.kind = Some(EventKind::Damage);
        self
    }
    pub fn by(mut self, actor: Sym) -> Self {
        self.actor = Some(actor);
        self
    }
    pub fn window(mut self, since: Millis, until: Millis) -> Self {
        self.since_ms = Some(since);
        self.until_ms = Some(until);
        self
    }
    pub fn with_tags(mut self, t: Tags) -> Self {
        self.tags_all |= t;
        self
    }
}

/// One row of a breakdown, keyed by ability.
#[derive(Debug, Clone)]
pub struct AbilityRow {
    pub ability: AbilityId,
    pub tags: Tags,
    pub total: u64,
    pub hits: u64,
    pub min: u64,
    pub max: u64,
    /// Hits at or within 1.5% of the ability's observed ceiling.
    pub full_power: u64,
    pub crits: u64,
    pub flags: Flags,
}

impl AbilityRow {
    pub fn mean(&self) -> f64 {
        if self.hits == 0 {
            0.0
        } else {
            self.total as f64 / self.hits as f64
        }
    }
    pub fn dps(&self, dur_ms: Millis) -> f64 {
        if dur_ms <= 0 {
            0.0
        } else {
            self.total as f64 / (dur_ms as f64 / 1000.0)
        }
    }
}

fn range_of(store: &Store, f: &Filter) -> std::ops::Range<usize> {
    match f.encounter.and_then(|id| store.encounter(id)) {
        Some(e) => e.range().start..e.range().end.min(store.len()),
        None => 0..store.len(),
    }
}

#[inline]
fn keep(store: &Store, i: usize, f: &Filter) -> bool {
    if let Some(k) = f.kind {
        if store.kind[i] != k {
            return false;
        }
    }
    if let Some(a) = f.actor {
        if store.actor[i] != a {
            return false;
        }
    }
    if let Some(t) = f.target {
        if store.target[i] != t {
            return false;
        }
    }
    if let Some(s) = f.since_ms {
        if store.ts[i] < s {
            return false;
        }
    }
    if let Some(u) = f.until_ms {
        if store.ts[i] > u {
            return false;
        }
    }
    if f.tags_all != 0 || f.tags_any != 0 {
        let t = store.abilities.tags(store.ability[i]);
        if f.tags_all != 0 && (t & f.tags_all) != f.tags_all {
            return false;
        }
        if f.tags_any != 0 && (t & f.tags_any) == 0 {
            return false;
        }
    }
    true
}

/// Breakdown by ability — the primary view.
///
/// Rows are abilities, not mechanisms, so `Backstab` sits beside a burn proc and
/// the two can be compared directly. Mechanism travels along as `tags` for
/// grouping, filtering and colour.
pub fn by_ability(store: &Store, f: &Filter) -> Vec<AbilityRow> {
    let mut acc: HashMap<AbilityId, AbilityRow> = HashMap::new();
    for i in range_of(store, f) {
        if !keep(store, i, f) {
            continue;
        }
        let a = store.ability[i];
        let amt = store.amount[i];
        let fl = store.flags[i];
        let r = acc.entry(a).or_insert(AbilityRow {
            ability: a,
            tags: store.abilities.tags(a),
            total: 0,
            hits: 0,
            min: u64::MAX,
            max: 0,
            full_power: 0,
            crits: 0,
            flags: 0,
        });
        r.total += amt;
        r.hits += 1;
        r.min = r.min.min(amt);
        r.max = r.max.max(amt);
        r.flags |= fl;
        if fl & crate::store::flag::CRITICAL != 0 {
            r.crits += 1;
        }
    }
    // Second pass, once over the range rather than once per row. Doing this
    // inside the loop above is not possible (the ceiling is only known after
    // the first pass) and doing it per row is quadratic in ability count.
    let mut cut: HashMap<AbilityId, u64> = HashMap::with_capacity(acc.len());
    for a in acc.keys() {
        let c = store.abilities.ceiling(*a);
        cut.insert(*a, if c > 0 { c - c / 66 } else { u64::MAX });
    }
    for i in range_of(store, f) {
        if !keep(store, i, f) {
            continue;
        }
        if let Some(&c) = cut.get(&store.ability[i]) {
            if store.amount[i] >= c {
                if let Some(r) = acc.get_mut(&store.ability[i]) {
                    r.full_power += 1;
                }
            }
        }
    }

    let mut v: Vec<AbilityRow> = acc.into_values().collect();
    for r in &mut v {
        if r.min == u64::MAX {
            r.min = 0;
        }
    }
    v.sort_by(|a, b| b.total.cmp(&a.total));
    v
}

/// Roll an ability breakdown up by mechanism. Derived from the same rows, so it
/// can never disagree with the ability view.
pub fn roll_up_by_tag(rows: &[AbilityRow]) -> Vec<(&'static str, u64, u64)> {
    let mut out = Vec::new();
    for (bit, name) in crate::ability::tag::ALL {
        let (mut total, mut hits) = (0u64, 0u64);
        for r in rows {
            if r.tags & bit != 0 {
                total += r.total;
                hits += r.hits;
            }
        }
        if hits > 0 {
            out.push((*name, total, hits));
        }
    }
    out.sort_by(|a, b| b.1.cmp(&a.1));
    out
}

/// Damage by actor. Same scan, different key.
pub fn by_actor(store: &Store, f: &Filter) -> Vec<(Sym, u64, u64)> {
    let mut acc: HashMap<Sym, (u64, u64)> = HashMap::new();
    for i in range_of(store, f) {
        if !keep(store, i, f) {
            continue;
        }
        let e = acc.entry(store.actor[i]).or_insert((0, 0));
        e.0 += store.amount[i];
        e.1 += 1;
    }
    let mut v: Vec<_> = acc.into_iter().map(|(k, (t, n))| (k, t, n)).collect();
    v.sort_by(|a, b| b.1.cmp(&a.1));
    v
}

pub fn total(store: &Store, f: &Filter) -> u64 {
    range_of(store, f)
        .filter(|&i| keep(store, i, f))
        .map(|i| store.amount[i])
        .sum()
}

/// DPS over a trailing window. No ring buffer, no eviction, no second copy of
/// the events — it is a filtered sum over a time range.
pub fn dps_window(store: &Store, f: &Filter, now: Millis, window_ms: Millis) -> f64 {
    let mut g = f.clone();
    g.since_ms = Some(now - window_ms + 1);
    g.until_ms = Some(now);
    total(store, &g) as f64 / (window_ms as f64 / 1000.0)
}
