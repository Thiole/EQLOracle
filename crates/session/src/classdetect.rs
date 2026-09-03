//! why: infers class configurations from what a character does -- no log
//! line states them. The rules live in docs/class-and-level-rules.md
//! (P1-P8); this file is their implementation, nothing more.
//!
//! One rolling evidence chain per entity. The unit of evidence is the
//! encounter (`Unit`), never the zone visit. Each class carries a
//! weight: a unit that supports it adds, a unit the entity fought in
//! without showing it subtracts, a zone line halves everything. A class
//! is confirmed once its unambiguous weight clears one bar, or its
//! elimination weight clears a stricter one -- nothing is ever forced.
//! A confirmed class that decays under the bar stays in the trio as a
//! prior until fresh evidence re-clears it or another class displaces
//! it. Contradictions (evidence no trio can hold) count per unit; three
//! in a row close the chain retroactively at the first of them and a
//! new chain starts there. Level floors ride the chain: a ding raises
//! every trio class below it, never lowers one.

use std::collections::{BTreeMap, BTreeSet, HashMap};

/// why: fixed game rule -- exactly three classes at once
pub const CLASS_COUNT: usize = 3;
/// why: the server's cap today (Spencer, 2026-09-03: "50, might change
/// later"); a wiki spell level above it is not this server's and proves nothing
pub const LEVEL_CAP: u8 = 50;

/// why: opaque evidence unit -- an encounter index, `None` before the first
pub type Unit = Option<usize>;
/// why: the old name, kept so callers read; the meaning is `Unit`
pub type ZoneVisit = Unit;
/// why: named to avoid tripping clippy's type_complexity on a bare tuple
pub type ConfiguredVisits = Vec<(Vec<String>, Vec<Unit>)>;

/// why: the numbers Spencer approved 2026-09-03 ("for now, will adjust
/// later if necessary") -- see docs Q33
const SUPPORT_GAIN: f32 = 1.0;
const WEIGHT_CAP: f32 = 3.0;
const UNSUPPORTED_LOSS: f32 = 0.5;
const CONTRADICT_LOSS: f32 = 1.0;
const ZONE_FACTOR: f32 = 0.5;
const UNAMBIGUOUS_BAR: f32 = 2.0;
const ELIMINATION_BAR: f32 = 3.0;
/// why: consecutive conflicting units before the chain closes (P5)
const CONTRADICTION_RUN: usize = 3;

/// why: `None` maps to 0 so units order as plain integers internally
fn key(u: Unit) -> usize {
    u.map_or(0, |i| i + 1)
}
fn unit(k: usize) -> Unit {
    if k == 0 {
        None
    } else {
        Some(k - 1)
    }
}

/// Everything one unit said about an entity, deduplicated.
#[derive(Debug, Clone, Default)]
struct UnitEvidence {
    unambiguous: BTreeSet<String>,
    pools: Vec<BTreeSet<String>>,
    /// why: (class, level) lists per cast -- a floor only when exactly
    /// one of the pairs' classes sits in the trio (P6)
    level_pairs: Vec<Vec<(String, u8)>>,
    dings: Vec<u8>,
    zone_lines: u32,
}

impl UnitEvidence {
    fn is_empty(&self) -> bool {
        self.unambiguous.is_empty() && self.pools.is_empty()
    }
    fn supports(&self, class: &str) -> bool {
        self.unambiguous.contains(class) || self.pools.iter().any(|p| p.contains(class))
    }
}

/// Why a chain ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainEnd {
    /// why: three consecutive conflicting units -- shown as "??" (P5)
    Contradiction,
    /// why: a loadout-swap signal from the app (P8)
    Swap,
}

/// The derived state of a chain after its committed units.
#[derive(Debug, Clone, Default)]
struct Derived {
    unamb: HashMap<String, f32>,
    elim: HashMap<String, f32>,
    /// why: every class that ever cleared a bar in this chain -- a prior
    /// once it decays, until re-cleared or displaced
    ever_confirmed: BTreeSet<String>,
    narrowing: Option<BTreeSet<String>>,
    floors: HashMap<String, u8>,
    max_ding: Option<u8>,
    conflict_run: Vec<usize>,
    units_seen: usize,
}

impl Derived {
    fn weight(&self, class: &str) -> f32 {
        self.unamb.get(class).copied().unwrap_or(0.0) + self.elim.get(class).copied().unwrap_or(0.0)
    }
    fn clears_bar(&self, class: &str) -> bool {
        self.unamb.get(class).copied().unwrap_or(0.0) >= UNAMBIGUOUS_BAR
            || self.elim.get(class).copied().unwrap_or(0.0) >= ELIMINATION_BAR
    }
    /// why: confirmed first (heaviest first), then priors by weight, at
    /// most CLASS_COUNT -- a prior at zero weight is what a newly
    /// confirmed class displaces
    fn trio(&self) -> (Vec<String>, Vec<String>) {
        let mut confirmed: Vec<String> = self
            .ever_confirmed
            .iter()
            .filter(|c| self.clears_bar(c))
            .cloned()
            .collect();
        confirmed.sort_by(|a, b| {
            self.weight(b)
                .partial_cmp(&self.weight(a))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.cmp(b))
        });
        confirmed.truncate(CLASS_COUNT);
        let mut prior: Vec<String> = self
            .ever_confirmed
            .iter()
            .filter(|c| !confirmed.contains(c))
            .cloned()
            .collect();
        prior.sort_by(|a, b| {
            self.weight(b)
                .partial_cmp(&self.weight(a))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.cmp(b))
        });
        prior.truncate(CLASS_COUNT - confirmed.len());
        (confirmed, prior)
    }
    fn trio_set(&self) -> BTreeSet<String> {
        let (c, p) = self.trio();
        c.into_iter().chain(p).collect()
    }

    /// why: one unit's evidence folded in, in rule order (P1, P2, P4-P6);
    /// returns whether the unit conflicted with the trio (P5)
    fn apply(&mut self, k: usize, ev: &UnitEvidence) -> bool {
        self.units_seen += 1;
        // P4: a zone line weakens everything before this unit's own evidence
        for _ in 0..ev.zone_lines {
            for w in self.unamb.values_mut().chain(self.elim.values_mut()) {
                *w *= ZONE_FACTOR;
            }
        }
        let before = self.trio_set();
        let full = before.len() == CLASS_COUNT;
        let mut conflict = false;

        // P2: unambiguous support
        for c in &ev.unambiguous {
            let w = self.unamb.entry(c.clone()).or_insert(0.0);
            *w = (*w + SUPPORT_GAIN).min(WEIGHT_CAP);
            if full && !before.contains(c) {
                conflict = true;
            }
        }
        // P2: elimination -- pools no trio class explains
        let unexplained: Vec<&BTreeSet<String>> = ev
            .pools
            .iter()
            .filter(|p| !p.iter().any(|c| before.contains(c)))
            .collect();
        if !unexplained.is_empty() {
            if full {
                conflict = true;
            } else if before.len() == CLASS_COUNT - 1 {
                let mut narrowed = self.narrowing.clone();
                for p in &unexplained {
                    narrowed = Some(match narrowed {
                        Some(n) => n.intersection(p).cloned().collect(),
                        None => (*p).clone(),
                    });
                }
                match narrowed {
                    Some(n) if n.is_empty() => {
                        conflict = true;
                        self.narrowing = None;
                    }
                    Some(n) => {
                        if n.len() == 1 {
                            let c = n.iter().next().cloned().unwrap_or_default();
                            let w = self.elim.entry(c).or_insert(0.0);
                            *w = (*w + SUPPORT_GAIN).min(WEIGHT_CAP);
                        }
                        self.narrowing = Some(n);
                    }
                    None => {}
                }
            }
        }
        // P1: unsupported in a unit the entity acted in shrinks; a
        // conflicting unit shrinks harder (only classes with any weight)
        if !ev.is_empty() {
            let loss = if conflict {
                CONTRADICT_LOSS
            } else {
                UNSUPPORTED_LOSS
            };
            let classes: Vec<String> = self
                .unamb
                .keys()
                .chain(self.elim.keys())
                .cloned()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            for c in classes {
                if ev.supports(&c) {
                    continue;
                }
                if let Some(w) = self.unamb.get_mut(&c) {
                    *w = (*w - loss).max(0.0);
                }
                if let Some(w) = self.elim.get_mut(&c) {
                    *w = (*w - loss).max(0.0);
                }
            }
        }
        // newly cleared bars join ever_confirmed and pick up the chain's
        // ding (P6) -- unless the trio was already full: a full trio only
        // changes through P5's close, never by quietly displacing a
        // decayed prior (that would rewrite the chain's whole history)
        let newly: Vec<String> = self
            .unamb
            .keys()
            .chain(self.elim.keys())
            .filter(|c| !full && self.clears_bar(c) && !self.ever_confirmed.contains(*c))
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        for c in newly {
            if let Some(d) = self.max_ding {
                let f = self.floors.entry(c.clone()).or_insert(0);
                *f = (*f).max(d);
            }
            self.ever_confirmed.insert(c);
        }
        // P6: dings raise every trio class below them
        let trio_now = self.trio_set();
        for &d in &ev.dings {
            self.max_ding = Some(self.max_ding.map_or(d, |m| m.max(d)));
            for c in &trio_now {
                let f = self.floors.entry(c.clone()).or_insert(0);
                *f = (*f).max(d);
            }
        }
        // P6: a spell only one trio class could cast raises that class
        for pairs in &ev.level_pairs {
            let mut fit = pairs.iter().filter(|(c, _)| trio_now.contains(c));
            if let (Some((c, l)), None) = (fit.next(), fit.next()) {
                if *l <= LEVEL_CAP {
                    let f = self.floors.entry(c.clone()).or_insert(0);
                    *f = (*f).max(*l);
                }
            }
        }
        // P5: consecutive conflicts
        if conflict {
            self.conflict_run.push(k);
        } else {
            self.conflict_run.clear();
        }
        conflict
    }
}

#[derive(Debug, Clone, Default)]
struct Chain {
    units: BTreeMap<usize, UnitEvidence>,
    committed: Derived,
    /// why: the unit still receiving evidence -- folded in provisionally
    /// on every query, committed when a later unit arrives
    current: Option<(usize, UnitEvidence)>,
    closed: Option<ChainEnd>,
    first: usize,
}

impl Chain {
    fn new(first: usize) -> Self {
        Chain {
            first,
            ..Default::default()
        }
    }
    fn last(&self) -> usize {
        self.current
            .as_ref()
            .map(|(k, _)| *k)
            .or_else(|| self.units.keys().next_back().copied())
            .unwrap_or(self.first)
    }
    fn derived(&self) -> Derived {
        let mut d = self.committed.clone();
        if let Some((k, ev)) = &self.current {
            d.apply(*k, ev);
        }
        d
    }
    fn evidence_mut(&mut self, k: usize) -> &mut UnitEvidence {
        match &mut self.current {
            Some((ck, _)) if *ck == k => {}
            Some((ck, _)) if *ck > k => {
                // why: late evidence for an already-committed unit folds
                // into the current one -- the chain sees it, ordering aside
            }
            _ => {
                if let Some((ck, ev)) = self.current.take() {
                    self.committed.apply(ck, &ev);
                    self.units.insert(ck, ev);
                }
                self.current = Some((k, UnitEvidence::default()));
            }
        }
        &mut self.current.as_mut().expect("current set").1
    }
    /// why: rebuilds from its units -- used after a split
    fn rebuild(&mut self) {
        let units = std::mem::take(&mut self.units);
        self.committed = Derived::default();
        self.current = None;
        for (k, ev) in units {
            self.committed.apply(k, &ev);
            self.units.insert(k, ev);
        }
    }
}

#[derive(Debug, Default)]
struct EntityState {
    chains: Vec<Chain>,
}

impl EntityState {
    /// why: L2 -- a class's floor is the character's, never lowered, so a
    /// trio swapped back in still reads what its classes reached before
    fn floor_of(&self, class: &str) -> Option<u8> {
        self.chains
            .iter()
            .filter_map(|c| c.derived().floors.get(class).copied())
            .max()
    }
}

impl EntityState {
    fn open_chain(&mut self, k: usize) -> &mut Chain {
        if self.chains.last().is_none_or(|c| c.closed.is_some()) {
            self.chains.push(Chain::new(k));
        }
        self.chains.last_mut().expect("just ensured")
    }
    fn chain_covering(&self, k: usize) -> Option<&Chain> {
        // why: the open chain covers everything after its first unit
        self.chains
            .iter()
            .rev()
            .find(|c| c.first <= k && (c.closed.is_none() || c.last() >= k))
    }
    /// why: P5 -- close at the first conflicting unit, replay the rest
    /// into a fresh chain that confirms on its own
    fn split_last(&mut self, at: usize, end: ChainEnd) {
        let Some(old) = self.chains.last_mut() else {
            return;
        };
        if let Some((ck, ev)) = old.current.take() {
            old.units.insert(ck, ev);
        }
        let moved: BTreeMap<usize, UnitEvidence> = old.units.split_off(&at);
        old.closed = Some(end);
        old.rebuild();
        let mut fresh = Chain::new(at);
        fresh.units = moved;
        fresh.rebuild();
        self.chains.push(fresh);
    }
}

/// A chain as the app reads it.
#[derive(Debug, Clone)]
pub struct ChainView {
    pub first: Unit,
    pub last: Unit,
    pub closed: Option<ChainEnd>,
    pub confirmed: Vec<String>,
    pub prior: Vec<String>,
    /// why: what the unresolved slot is stuck between (Q34) -- the
    /// current elimination narrowing, else every class with weight
    pub candidates: Vec<String>,
    pub floors: Vec<(String, u8)>,
    pub max_ding: Option<u8>,
    pub units: usize,
    pub weights: Vec<(String, f32)>,
    pub conflicts: usize,
}

impl ChainView {
    /// why: the trio as shown -- confirmed then priors
    pub fn trio(&self) -> Vec<String> {
        let mut v = self.confirmed.clone();
        v.extend(self.prior.iter().cloned());
        v
    }
    pub fn is_full(&self) -> bool {
        self.trio().len() == CLASS_COUNT
    }
}

/// why: per-entity chains, never reset/decayed/evicted except by the rules
#[derive(Debug, Default)]
pub struct Detector {
    by_entity: HashMap<u32, EntityState>,
}

impl Detector {
    /// why: one cast/song/stance/skill line's class pool for `entity` in `unit`
    pub fn observe_cast(&mut self, entity: u32, unit: Unit, classes: &[String]) {
        if classes.is_empty() {
            return;
        }
        let k = key(unit);
        let state = self.by_entity.entry(entity).or_default();
        let chain = state.open_chain(k);
        let ev = chain.evidence_mut(k);
        if classes.len() == 1 {
            ev.unambiguous.insert(classes[0].clone());
        } else {
            let pool: BTreeSet<String> = classes.iter().cloned().collect();
            if !ev.pools.contains(&pool) {
                ev.pools.push(pool);
            }
        }
        Self::settle(state);
    }

    /// why: a cast's (class, level) pairs -- P6's spell floor
    pub fn observe_spell_levels(&mut self, entity: u32, unit: Unit, pairs: &[(String, u8)]) {
        if pairs.is_empty() {
            return;
        }
        let k = key(unit);
        let state = self.by_entity.entry(entity).or_default();
        let ev = state.open_chain(k).evidence_mut(k);
        let v = pairs.to_vec();
        if !ev.level_pairs.contains(&v) {
            ev.level_pairs.push(v);
        }
    }

    /// why: P4 -- a zone line weakens, never breaks
    pub fn observe_zone_line(&mut self, entity: u32, unit: Unit) {
        let k = key(unit);
        let state = self.by_entity.entry(entity).or_default();
        state.open_chain(k).evidence_mut(k).zone_lines += 1;
    }

    /// why: P6 -- a ding is the trio's lowest; it raises what's below it
    pub fn observe_ding(&mut self, entity: u32, unit: Unit, level: u8) {
        let k = key(unit);
        let state = self.by_entity.entry(entity).or_default();
        state.open_chain(k).evidence_mut(k).dings.push(level);
    }

    /// why: P8 -- a swap signal closes the chain now; evidence from
    /// `unit` on belongs to a fresh one
    pub fn close_chain(&mut self, entity: u32, unit: Unit) {
        let k = key(unit);
        let state = self.by_entity.entry(entity).or_default();
        let Some(last) = state.chains.last_mut() else {
            return;
        };
        if last.closed.is_some() {
            return;
        }
        if last.first >= k {
            // why: nothing before the split point -- the chain simply restarts
            let fresh = Chain::new(k);
            *last = fresh;
            return;
        }
        state.split_last(k, ChainEnd::Swap);
    }

    /// why: P5's close, checked after every observation on the open chain
    fn settle(state: &mut EntityState) {
        let Some(last) = state.chains.last_mut() else {
            return;
        };
        if last.closed.is_some() {
            return;
        }
        // why: the current unit counts too -- the third conflicting unit
        // closes the chain as soon as it conflicts, not one unit later
        let run = last.derived().conflict_run;
        if run.len() >= CONTRADICTION_RUN {
            if let Some((ck, ev)) = last.current.take() {
                last.committed.apply(ck, &ev);
                last.units.insert(ck, ev);
            }
            state.split_last(run[0], ChainEnd::Contradiction);
        }
    }

    fn view(state: &EntityState, chain: &Chain) -> ChainView {
        let d = chain.derived();
        let (confirmed, prior) = d.trio();
        let trio: BTreeSet<&String> = confirmed.iter().chain(prior.iter()).collect();
        let candidates: Vec<String> = if trio.len() < CLASS_COUNT {
            match &d.narrowing {
                Some(n) if !n.is_empty() => n.iter().cloned().collect(),
                _ => {
                    let mut v: Vec<(String, f32)> = d
                        .unamb
                        .keys()
                        .chain(d.elim.keys())
                        .filter(|c| !trio.contains(c))
                        .map(|c| (c.clone(), d.weight(c)))
                        .filter(|(_, w)| *w > 0.0)
                        .collect::<BTreeMap<_, _>>()
                        .into_iter()
                        .collect();
                    v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
                    v.into_iter().map(|(c, _)| c).collect()
                }
            }
        } else {
            Vec::new()
        };
        let mut weights: Vec<(String, f32)> = d
            .unamb
            .keys()
            .chain(d.elim.keys())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(|c| (c.clone(), d.weight(c)))
            .filter(|(_, w)| *w > 0.0)
            .collect();
        weights.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        let mut floors: Vec<(String, u8)> = trio
            .iter()
            .filter_map(|c| state.floor_of(c).map(|l| ((*c).clone(), l)))
            .collect();
        floors.sort();
        ChainView {
            first: unit(chain.first),
            last: unit(chain.last()),
            closed: chain.closed,
            confirmed,
            prior,
            candidates,
            floors,
            max_ding: d.max_ding,
            units: d.units_seen,
            weights,
            conflicts: d.conflict_run.len(),
        }
    }

    /// why: the chain that covers `unit`, as the app reads it
    pub fn chain_at(&self, entity: u32, unit: Unit) -> Option<ChainView> {
        let state = self.by_entity.get(&entity)?;
        state
            .chain_covering(key(unit))
            .map(|c| Self::view(state, c))
    }

    /// why: every chain, oldest first
    pub fn chains(&self, entity: u32) -> Vec<ChainView> {
        self.by_entity
            .get(&entity)
            .map(|s| s.chains.iter().map(|c| Self::view(s, c)).collect())
            .unwrap_or_default()
    }

    /// why: the trio (confirmed then priors) of the chain covering `unit`
    pub fn configuration_of_visit(&self, entity: u32, unit: Unit) -> Vec<String> {
        self.chain_at(entity, unit)
            .map(|v| {
                let mut t = v.trio();
                t.sort();
                t
            })
            .unwrap_or_default()
    }

    /// why: every distinct full trio, most-units-first
    pub fn configurations_of(&self, entity: u32) -> Vec<(Vec<String>, usize)> {
        let mut counts: HashMap<Vec<String>, usize> = HashMap::new();
        for v in self.chains(entity) {
            if !v.is_full() {
                continue;
            }
            let mut t = v.trio();
            t.sort();
            *counts.entry(t).or_insert(0) += v.units.max(1);
        }
        let mut out: Vec<(Vec<String>, usize)> = counts.into_iter().collect();
        out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        out
    }

    /// why: full chains as (trio, units); partial chains' units as unresolved
    pub fn visits_by_resolved_configuration(&self, entity: u32) -> (ConfiguredVisits, Vec<Unit>) {
        let Some(state) = self.by_entity.get(&entity) else {
            return (Vec::new(), Vec::new());
        };
        let mut full: Vec<(Vec<String>, Vec<Unit>)> = Vec::new();
        let mut unresolved: Vec<Unit> = Vec::new();
        for chain in &state.chains {
            let v = Self::view(state, chain);
            let mut units: Vec<Unit> = chain.units.keys().map(|k| unit(*k)).collect();
            if let Some((k, _)) = &chain.current {
                units.push(unit(*k));
            }
            if v.is_full() {
                let mut t = v.trio();
                t.sort();
                full.push((t, units));
            } else {
                unresolved.extend(units);
            }
        }
        full.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then_with(|| a.0.cmp(&b.0)));
        (full, unresolved)
    }

    /// Every entity with any evidence at all, ever.
    pub fn known_entities(&self) -> impl Iterator<Item = u32> + '_ {
        self.by_entity.keys().copied()
    }
}
