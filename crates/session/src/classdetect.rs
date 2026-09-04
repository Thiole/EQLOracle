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
    /// why: a /who row states the trio outright -- ground truth for the
    /// chain it printed in, the one evidence kind that needs no bar
    who: Option<(u8, Vec<String>)>,
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
}

/// Why a chain ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainEnd {
    /// why: three consecutive conflicting units -- shown as "??" (P5)
    Contradiction,
    /// why: a loadout-swap signal from the app (P8)
    Swap,
}

/// why: every class the game has -- trios are enumerated over these.
/// "Shadowknight" (one real pack spelling) folds onto "Shadow Knight".
pub const CLASSES: [&str; 16] = [
    "Bard",
    "Beastlord",
    "Berserker",
    "Cleric",
    "Druid",
    "Enchanter",
    "Magician",
    "Monk",
    "Necromancer",
    "Paladin",
    "Ranger",
    "Rogue",
    "Shadow Knight",
    "Shaman",
    "Warrior",
    "Wizard",
];

fn class_ix(name: &str) -> Option<usize> {
    let name = if name.eq_ignore_ascii_case("shadowknight") {
        "Shadow Knight"
    } else {
        name
    };
    CLASSES.iter().position(|c| *c == name)
}

/// why: all C(16,3) = 560 trios, built once; scores index into this
fn trios() -> &'static [[usize; 3]] {
    static T: std::sync::OnceLock<Vec<[usize; 3]>> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let n = CLASSES.len();
        let mut v = Vec::with_capacity(560);
        for a in 0..n {
            for b in a + 1..n {
                for c in b + 1..n {
                    v.push([a, b, c]);
                }
            }
        }
        v
    })
}

/// why: a trio is the best guess once its rolling score clears this
const TRIO_BAR: f32 = 2.0;

/// The derived state of a chain after its committed units.
///
/// why: Spencer's "attribute all data at once" -- every unit is scored
/// against every possible trio, and the row shows the intersection of
/// the trios that lead. Classes still carry their own rolling weights,
/// which are the P2 bars (2 units class-only, 3 units of pools) a class
/// must clear before it counts as confirmed inside that intersection.
#[derive(Debug, Clone)]
struct Derived {
    /// why: rolling score per trio, `trios()` order
    scores: Vec<f32>,
    unamb: HashMap<String, f32>,
    pooled: HashMap<String, f32>,
    /// why: every class that ever counted as confirmed in this chain --
    /// a prior once it drops out, until re-cleared or the chain closes
    ever_confirmed: BTreeSet<String>,
    confirmed_now: BTreeSet<String>,
    /// why: what the leading trios disagree on -- the open slot's candidates
    candidates: BTreeSet<String>,
    floors: HashMap<String, u8>,
    max_ding: Option<u8>,
    conflict_run: Vec<usize>,
    units_seen: usize,
    who: Option<(u8, Vec<String>)>,
    /// why: what every leading trio agrees on, before the per-class bar --
    /// the honest best guess after a single sighting, which the ally
    /// table shows with a "?" until the bar is cleared
    leading_now: BTreeSet<String>,
}

impl Default for Derived {
    fn default() -> Self {
        Derived {
            scores: vec![0.0; trios().len()],
            unamb: HashMap::new(),
            pooled: HashMap::new(),
            ever_confirmed: BTreeSet::new(),
            confirmed_now: BTreeSet::new(),
            candidates: BTreeSet::new(),
            floors: HashMap::new(),
            max_ding: None,
            conflict_run: Vec::new(),
            units_seen: 0,
            who: None,
            leading_now: BTreeSet::new(),
        }
    }
}

impl Derived {
    fn weight(&self, class: &str) -> f32 {
        self.unamb.get(class).copied().unwrap_or(0.0)
            + self.pooled.get(class).copied().unwrap_or(0.0)
    }
    /// why: P2's bars -- 2 units of class-only evidence, or 3 of pools
    fn clears_bar(&self, class: &str) -> bool {
        self.unamb.get(class).copied().unwrap_or(0.0) >= UNAMBIGUOUS_BAR
            || self.pooled.get(class).copied().unwrap_or(0.0) >= ELIMINATION_BAR
    }
    /// why: confirmed first (heaviest first), then priors by weight, at
    /// most CLASS_COUNT
    /// why: the INFERENCE only -- a /who row is applied by `ChainView::trio`
    /// on top, so the two stay separately readable (the ally_class_check
    /// probe scores one against the other)
    fn trio(&self) -> (Vec<String>, Vec<String>) {
        let by_weight = |a: &String, b: &String| {
            self.weight(b)
                .partial_cmp(&self.weight(a))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.cmp(b))
        };
        let mut confirmed: Vec<String> = self.confirmed_now.iter().cloned().collect();
        confirmed.sort_by(by_weight);
        confirmed.truncate(CLASS_COUNT);
        let mut prior: Vec<String> = self
            .ever_confirmed
            .iter()
            .filter(|c| !confirmed.contains(c))
            .cloned()
            .collect();
        prior.sort_by(by_weight);
        prior.truncate(CLASS_COUNT - confirmed.len());
        (confirmed, prior)
    }
    fn trio_set(&self) -> BTreeSet<String> {
        let (c, p) = self.trio();
        c.into_iter().chain(p).collect()
    }
    /// why: "all data at once" -- the trios that fit the whole chain
    /// best share the top score; their intersection is the best guess,
    /// what they disagree on is the open slot's candidates
    fn leading(&self) -> Vec<usize> {
        let max = self.scores.iter().cloned().fold(f32::MIN, f32::max);
        if max <= 0.0 {
            return Vec::new();
        }
        (0..self.scores.len())
            .filter(|&i| (self.scores[i] - max).abs() < 1e-6)
            .collect()
    }
    fn at_bar(&self) -> bool {
        self.scores.iter().cloned().fold(f32::MIN, f32::max) >= TRIO_BAR
    }
    fn consistent(t: &[usize; 3], unamb: &[usize], pools: &[Vec<usize>]) -> bool {
        unamb.iter().all(|c| t.contains(c)) && pools.iter().all(|p| p.iter().any(|c| t.contains(c)))
    }

    /// why: one unit's evidence folded in, in rule order (P1, P2, P4-P6);
    /// returns whether the unit conflicted with the leading trios (P5)
    fn apply(&mut self, k: usize, ev: &UnitEvidence) -> bool {
        self.units_seen += 1;
        // why: ground truth -- a /who row's trio IS the answer for this
        // chain, and its level floors every class in it
        if let Some((level, trio)) = &ev.who {
            self.who = Some((*level, trio.clone()));
            for c in trio {
                let f = self.floors.entry(c.clone()).or_insert(0);
                *f = (*f).max(*level);
            }
        }
        // P4: a zone line weakens everything before this unit's own evidence
        for _ in 0..ev.zone_lines {
            for w in self.scores.iter_mut() {
                *w *= ZONE_FACTOR;
            }
            for w in self.unamb.values_mut().chain(self.pooled.values_mut()) {
                *w *= ZONE_FACTOR;
            }
        }
        let before = self.trio_set();
        let full = before.len() == CLASS_COUNT;
        let mut conflict = false;

        if !ev.is_empty() {
            let unamb: Vec<usize> = ev.unambiguous.iter().filter_map(|c| class_ix(c)).collect();
            let pools: Vec<Vec<usize>> = ev
                .pools
                .iter()
                .map(|p| p.iter().filter_map(|c| class_ix(c)).collect::<Vec<_>>())
                .filter(|p: &Vec<usize>| !p.is_empty())
                .collect();
            // P5: a full trio in effect (confirmed plus priors) conflicts
            // with any unit it cannot hold; short of one, the unit
            // conflicts when no trio at the bar can hold it
            if full {
                let held: Vec<usize> = before.iter().filter_map(|c| class_ix(c)).collect();
                if held.len() == CLASS_COUNT {
                    let t = [held[0], held[1], held[2]];
                    if !Self::consistent(&t, &unamb, &pools) {
                        conflict = true;
                    }
                }
            } else {
                let leading = self.leading();
                if self.at_bar()
                    && !leading
                        .iter()
                        .any(|&i| Self::consistent(&trios()[i], &unamb, &pools))
                {
                    conflict = true;
                }
            }
            // P1 as "all at once": every trio scored against this unit --
            // a fit adds one, a miss costs one, nothing is capped, so the
            // trios that fit the whole chain stay ahead of any that missed
            // even once; only a zone line (P4) scales everything down
            for (i, t) in trios().iter().enumerate() {
                let w = &mut self.scores[i];
                if Self::consistent(t, &unamb, &pools) {
                    *w += SUPPORT_GAIN;
                } else {
                    *w -= CONTRADICT_LOSS;
                }
            }
            // P2 bars per class: units of class-only evidence, units of pools
            for c in &ev.unambiguous {
                *self.unamb.entry(c.clone()).or_insert(0.0) += SUPPORT_GAIN;
            }
            let in_pools: BTreeSet<&String> = ev.pools.iter().flatten().collect();
            for c in &in_pools {
                *self.pooled.entry((*c).clone()).or_insert(0.0) += SUPPORT_GAIN;
            }
        }

        // the best guess: intersection of the leading trios, gated by the
        // per-class bars; a full trio only changes through P5's close
        let leading = if self.at_bar() {
            self.leading()
        } else {
            Vec::new()
        };
        let mut inter: Option<BTreeSet<usize>> = None;
        let mut union: BTreeSet<usize> = BTreeSet::new();
        for &i in &leading {
            let t: BTreeSet<usize> = trios()[i].iter().copied().collect();
            union.extend(t.iter().copied());
            inter = Some(match inter {
                Some(x) => x.intersection(&t).copied().collect(),
                None => t,
            });
        }
        let inter = inter.unwrap_or_default();
        let mut confirmed_now: BTreeSet<String> = BTreeSet::new();
        for &ix in &inter {
            let c = CLASSES[ix].to_string();
            if !self.clears_bar(&c) {
                continue;
            }
            if full && !before.contains(&c) {
                continue;
            }
            confirmed_now.insert(c);
        }
        for c in &confirmed_now {
            if !self.ever_confirmed.contains(c) {
                if let Some(d) = self.max_ding {
                    let f = self.floors.entry(c.clone()).or_insert(0);
                    *f = (*f).max(d);
                }
                self.ever_confirmed.insert(c.clone());
            }
        }
        self.confirmed_now = confirmed_now;
        let (cand_union, cand_inter) = if leading.is_empty() {
            // why: below the bar the top trios still say what is forming
            let mut u: BTreeSet<usize> = BTreeSet::new();
            let mut i: Option<BTreeSet<usize>> = None;
            for &ix in &self.leading() {
                let t: BTreeSet<usize> = trios()[ix].iter().copied().collect();
                u.extend(t.iter().copied());
                i = Some(match i {
                    Some(x) => x.intersection(&t).copied().collect(),
                    None => t,
                });
            }
            (u, i.unwrap_or_default())
        } else {
            (union, inter)
        };
        self.candidates = cand_union
            .iter()
            .filter(|ix| !cand_inter.contains(ix))
            .map(|&ix| CLASSES[ix].to_string())
            .collect();
        // why: what every top trio agrees on, bar cleared or not -- one
        // sighting of a class-only spell already answers with that class
        self.leading_now = cand_inter
            .iter()
            .map(|&ix| CLASSES[ix].to_string())
            .collect();

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
    /// why: a closed chain frozen to its result once a zone is done --
    /// its evidence and score table are dropped (prediction tables are
    /// extraneous data, Spencer), only what it concluded stays
    frozen: Option<(ChainView, Vec<usize>)>,
}

impl Chain {
    fn new(first: usize) -> Self {
        Chain {
            first,
            ..Default::default()
        }
    }
    fn last(&self) -> usize {
        if let Some((v, _)) = &self.frozen {
            return key(v.last);
        }
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
            .filter_map(|c| match &c.frozen {
                Some((v, _)) => v.floors.iter().find(|(n, _)| n == class).map(|(_, l)| *l),
                None => c.derived().floors.get(class).copied(),
            })
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
    /// why: a /who row printed in this chain -- its trio is the answer and
    /// its level is ground truth (your own row gets the same treatment)
    pub who: Option<(u8, Vec<String>)>,
    /// why: what the leading trios already agree on, bar or no bar -- a
    /// first sighting's honest guess
    pub leading: Vec<String>,
    pub floors: Vec<(String, u8)>,
    pub max_ding: Option<u8>,
    pub units: usize,
    pub weights: Vec<(String, f32)>,
    pub conflicts: usize,
}

impl ChainView {
    /// why: the trio as shown -- a /who row is ground truth and wins;
    /// otherwise what the evidence confirmed, then anything carried as a prior
    pub fn trio(&self) -> Vec<String> {
        if let Some((_, trio)) = &self.who {
            return trio.clone();
        }
        let mut v = self.confirmed.clone();
        v.extend(self.prior.iter().cloned());
        v
    }

    /// why: what the evidence alone says, ignoring any /who row -- lets a
    /// probe score inference against ground truth. Falls back to what the
    /// leading trios agree on, so one sighting still answers.
    pub fn inferred(&self) -> Vec<String> {
        let mut v = self.confirmed.clone();
        v.extend(self.prior.iter().cloned());
        if v.is_empty() {
            v = self.leading.clone();
        }
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

    /// why: a /who row for `entity`: its trio is this chain's answer and
    /// its level is ground truth. Same call for an ally and for you --
    /// one model, the self side just has more evidence feeding it.
    pub fn observe_who(&mut self, entity: u32, unit: Unit, level: u8, trio: Vec<String>) {
        if trio.is_empty() {
            return;
        }
        let k = key(unit);
        let state = self.by_entity.entry(entity).or_default();
        state.open_chain(k).evidence_mut(k).who = Some((level, trio));
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
        // why: the COMMITTED run only -- deriving here re-applied the open
        // unit (a 560-trio pass and a full clone of the score table) on
        // every single observation, and with allies feeding the same
        // detector that ran millions of times: a real 14s -> 100s backfill
        // regression. A unit's evidence isn't final until it ends anyway,
        // so the split lands when the third conflicting unit commits.
        let run = &last.committed.conflict_run;
        if run.len() >= CONTRADICTION_RUN {
            let at = run[0];
            state.split_last(at, ChainEnd::Contradiction);
        }
    }

    fn view(state: &EntityState, chain: &Chain) -> ChainView {
        if let Some((v, _)) = &chain.frozen {
            return v.clone();
        }
        let d = chain.derived();
        let (confirmed, prior) = d.trio();
        let trio: BTreeSet<&String> = confirmed.iter().chain(prior.iter()).collect();
        let candidates: Vec<String> = if trio.len() < CLASS_COUNT {
            d.candidates
                .iter()
                .filter(|c| !trio.contains(c))
                .cloned()
                .collect()
        } else {
            Vec::new()
        };
        let mut weights: Vec<(String, f32)> = d
            .unamb
            .keys()
            .chain(d.pooled.keys())
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
            who: d.who.clone(),
            leading: d.leading_now.iter().cloned().collect(),
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

    /// why: once a zone is done, every closed chain keeps only its result
    /// -- the per-unit evidence, pool history and 560-trio score table go
    pub fn freeze_closed(&mut self, entity: u32) {
        let Some(state) = self.by_entity.get_mut(&entity) else {
            return;
        };
        let n = state.chains.len();
        for i in 0..n {
            if state.chains[i].closed.is_none() || state.chains[i].frozen.is_some() {
                continue;
            }
            let view = Self::view(state, &state.chains[i]);
            let units: Vec<usize> = state.chains[i].units.keys().copied().collect();
            let c = &mut state.chains[i];
            c.frozen = Some((view, units));
            c.units = BTreeMap::new();
            c.committed = Derived::default();
            c.current = None;
        }
    }

    /// why: cheap "did a new chain start" probe for callers that hold
    /// per-chain state of their own (the stance in effect)
    pub fn chain_count(&self, entity: u32) -> usize {
        self.by_entity.get(&entity).map_or(0, |s| s.chains.len())
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
            let mut units: Vec<Unit> = match &chain.frozen {
                Some((_, keys)) => keys.iter().map(|k| unit(*k)).collect(),
                None => chain.units.keys().map(|k| unit(*k)).collect(),
            };
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
