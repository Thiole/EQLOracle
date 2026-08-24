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
    /// Zone difficulty tier (0-4) a row was recorded at -- see
    /// `Store::tier`'s doc for why this is an opaque app-supplied byte
    /// rather than anything `eqlp-store` interprets.
    pub tier: Option<u8>,
}

impl Filter {
    pub fn encounter(id: EncounterId) -> Self {
        Filter {
            encounter: Some(id),
            ..Default::default()
        }
    }
    pub fn damage(mut self) -> Self {
        self.kind = Some(EventKind::Damage);
        self
    }
    pub fn kind(mut self, kind: EventKind) -> Self {
        self.kind = Some(kind);
        self
    }
    pub fn by(mut self, actor: Sym) -> Self {
        self.actor = Some(actor);
        self
    }
    pub fn target(mut self, target: Sym) -> Self {
        self.target = Some(target);
        self
    }
    pub fn tier(mut self, tier: u8) -> Self {
        self.tier = Some(tier);
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
    /// Sum of just the crit hits' own damage -- `total - crit_total` is
    /// the non-crit sum, `crit_total / crits` the average crit. Kept
    /// separate from `total` rather than derived from `max`/`crits` alone
    /// because a row can have several distinct crit values, not one.
    pub crit_total: u64,
    /// Swings of this same ability (`Punch`, `Slash`, ...) that dealt zero
    /// damage because the target fully avoided them -- broken out by how,
    /// not folded into `hits`/`total`/`min`/`max`, which stay exactly
    /// "landed swings only" the same way they always have. See
    /// `flag::MITIGATED`'s own doc for why this lives on the *same*
    /// ability row rather than a separate synthetic one.
    pub missed: u64,
    pub blocked: u64,
    pub dodged: u64,
    pub parried: u64,
    pub flags: Flags,
}

impl AbilityRow {
    /// Every swing attempted against/by this ability, landed or not --
    /// the honest denominator for a real hit rate (`hits as f64 /
    /// attempts as f64`), not `hits` alone.
    pub fn attempts(&self) -> u64 {
        self.hits + self.missed + self.blocked + self.dodged + self.parried
    }
    pub fn mean(&self) -> f64 {
        if self.hits == 0 {
            0.0
        } else {
            self.total as f64 / self.hits as f64
        }
    }
    /// Average of just the non-crit hits. Falls back to the overall mean
    /// when every hit crit (or there's no crit tracking for this row --
    /// see `by_target_and_ability`'s doc) so callers never divide by zero.
    pub fn avg_normal(&self) -> f64 {
        let normal_hits = self.hits.saturating_sub(self.crits);
        if normal_hits == 0 {
            self.mean()
        } else {
            (self.total - self.crit_total) as f64 / normal_hits as f64
        }
    }
    /// Average of just the crit hits. Zero when this ability never crit.
    pub fn avg_crit(&self) -> f64 {
        if self.crits == 0 {
            0.0
        } else {
            self.crit_total as f64 / self.crits as f64
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
    if let Some(enc) = f.encounter {
        // `range_of` narrows the scan to this encounter's index range, but
        // a range is not a membership test: when two fights' events
        // interleave in append order (routine once several encounters
        // overlap in time -- other combat nearby, a pet's own actions),
        // a row that falls inside the range can still belong to a
        // different encounter. The `enc` column is what actually says
        // whose row this is; check it, not just the range that got us here.
        if store.enc[i] != enc.0 {
            return false;
        }
    }
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
    if let Some(t) = f.tier {
        if store.tier[i] != t {
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
            crit_total: 0,
            missed: 0,
            blocked: 0,
            dodged: 0,
            parried: 0,
            flags: 0,
        });
        r.flags |= fl;
        // A fully-mitigated swing (see `flag::MITIGATED`'s own doc) never
        // landed -- counted by how, kept out of hits/total/min/max, which
        // stay "landed swings only" exactly as before this existed.
        if fl & crate::store::flag::MISSED != 0 {
            r.missed += 1;
        } else if fl & crate::store::flag::BLOCKED != 0 {
            r.blocked += 1;
        } else if fl & crate::store::flag::DODGED != 0 {
            r.dodged += 1;
        } else if fl & crate::store::flag::PARRIED != 0 {
            r.parried += 1;
        } else {
            r.total += amt;
            r.hits += 1;
            r.min = r.min.min(amt);
            r.max = r.max.max(amt);
            if fl & crate::store::flag::CRITICAL != 0 {
                r.crits += 1;
                r.crit_total += amt;
            }
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
    v.sort_by_key(|b| std::cmp::Reverse(b.total));
    v
}

/// Every row of one `kind`, grouped by `target` and then by `ability`, in a
/// single O(store length) pass -- regardless of how many distinct targets
/// exist. Exists for callers that need a breakdown "per target" over the
/// *whole* store (every mob type's loot, say): calling `by_ability` once
/// per target with a `.target(sym)` filter would cost one full store scan
/// *per target*, turning an O(n) question into O(targets * n) -- fine for
/// one target, a real cost once there are hundreds (`crate` doesn't gate
/// this behind `Filter` the way `by_ability` does, since "every target at
/// once" is a different access pattern, not another predicate to combine).
///
/// `full_power`/`crits`/`crit_total`/`missed`/`blocked`/`dodged`/`parried`
/// are left at their zero defaults: nothing that calls this needs them
/// (they exist so `AbilityRow` doesn't need two shapes), and computing
/// `full_power` needs the ceiling-based second pass `by_ability` does,
/// which would reintroduce the same per-target cost this function exists
/// to avoid.
pub fn by_target_and_ability(store: &Store, kind: EventKind) -> HashMap<Sym, Vec<AbilityRow>> {
    let mut acc: HashMap<(Sym, AbilityId), AbilityRow> = HashMap::new();
    for i in 0..store.len() {
        if store.kind[i] != kind {
            continue;
        }
        let key = (store.target[i], store.ability[i]);
        let amt = store.amount[i];
        let r = acc.entry(key).or_insert(AbilityRow {
            ability: store.ability[i],
            tags: store.abilities.tags(store.ability[i]),
            total: 0,
            hits: 0,
            min: u64::MAX,
            max: 0,
            full_power: 0,
            crits: 0,
            crit_total: 0,
            missed: 0,
            blocked: 0,
            dodged: 0,
            parried: 0,
            flags: 0,
        });
        r.total += amt;
        r.hits += 1;
        r.min = r.min.min(amt);
        r.max = r.max.max(amt);
        r.flags |= store.flags[i];
    }

    let mut by_target: HashMap<Sym, Vec<AbilityRow>> = HashMap::new();
    for ((target, _ability), mut row) in acc {
        if row.min == u64::MAX {
            row.min = 0;
        }
        by_target.entry(target).or_default().push(row);
    }
    by_target
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
    out.sort_by_key(|b| std::cmp::Reverse(b.1));
    out
}

/// Damage by actor. Same scan, different key.
/// `(actor, total damage, hits, crits)` per actor. `crits` is a subset of
/// `hits`, same convention as `AbilityRow` -- a caller wanting crit chance
/// divides the two rather than being handed a pre-computed percentage, so
/// it stays comparable across selections without re-deriving the count.
pub fn by_actor(store: &Store, f: &Filter) -> Vec<(Sym, u64, u64, u64)> {
    let mut acc: HashMap<Sym, (u64, u64, u64)> = HashMap::new();
    for i in range_of(store, f) {
        if !keep(store, i, f) {
            continue;
        }
        let e = acc.entry(store.actor[i]).or_insert((0, 0, 0));
        e.0 += store.amount[i];
        e.1 += 1;
        if store.flags[i] & crate::store::flag::CRITICAL != 0 {
            e.2 += 1;
        }
    }
    let mut v: Vec<_> = acc.into_iter().map(|(k, (t, n, c))| (k, t, n, c)).collect();
    v.sort_by_key(|b| std::cmp::Reverse(b.1));
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
