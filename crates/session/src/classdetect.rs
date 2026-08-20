//! Inferring an entity's class configuration(s) from what they cast.
//!
//! Confirmed facts this is built around:
//! - No log line ever states a character's class, race, or loadout --
//!   every cast is the only evidence available.
//! - Every character above level 10 plays exactly `CLASS_COUNT` classes
//!   simultaneously. A fixed-cardinality fact about this game, not a
//!   confidence question to estimate.
//! - A loadout can be swapped at will, any time, while in town.
//!
//! ## Design: confirmed classes, grouped per zone visit
//!
//! Two earlier versions of this module tried to answer "what is this
//! entity playing" as a *single* rolling value -- first a decayed weight
//! per class, then a size-`CLASS_COUNT` LRU set. Both have the same flaw:
//! a real configuration that's used only occasionally (a Shadow Knight
//! loadout kept for one specific fight, say) gets crowded out by whatever
//! is played *most*, and then it's gone from the picture entirely, even
//! though it's genuinely real and was genuinely used. There is no single
//! "current" configuration to report when a player legitimately keeps
//! several around and switches between them -- the honest answer is a
//! *set of configurations*, not one rolling guess.
//!
//! So evidence is grouped by **zone visit** instead: within one visit, a
//! player's loadout is stable (a swap needs town, which is a zone
//! boundary in practice), so the classes confirmed during one visit are
//! one real, coherent configuration. Different visits can and often do
//! land on the same configuration (most of a session, say); some visits
//! land on a different one entirely (that Shadow Knight fight). Nothing is
//! ever evicted or decayed -- every zone visit's confirmed set is kept, so
//! `configurations_of` can report every distinct configuration this
//! entity has ever actually played, each with how many visits used it,
//! rather than only the loudest one.
//!
//! ## Confirming a class: unambiguous evidence, or elimination
//!
//! A class enters a visit's confirmed set two ways:
//!
//! 1. **Unambiguous cast.** A spell with exactly one eligible class
//!    (`Alacrity` doesn't qualify -- Enchanter *and* Shaman; `Fire Bolt`
//!    does -- Wizard alone) confirms that class -- once its own class has
//!    reached `MIN_UNAMBIGUOUS_CASTS` occurrences for this entity, ever.
//!    Not one and done: checked against a real log, a single vendor-sold
//!    off-class spell -- real, wiki-exclusive to Druid, bought and cast
//!    exactly once, never repeated -- read as permanent proof of a class
//!    that was never actually played, while every class the player
//!    genuinely played showed multiple distinct qualifying casts or clear
//!    repeats. Two is the bar; see that constant's own doc.
//! 2. **Elimination.** Once a visit has confirmed exactly `CLASS_COUNT - 1`
//!    classes, exactly one slot is provably still open -- not "probably",
//!    provably, from the same fixed-cardinality rule this whole module
//!    already leans on. Every ambiguous cast that lands from then on whose
//!    candidates don't include an already-confirmed class must be evidence
//!    about *that* slot specifically, since there is nowhere else for it
//!    to belong. Intersecting those candidate sets as they arrive narrows
//!    the possibilities; when the intersection reaches exactly one class,
//!    that's a proof by elimination, not a guess, and it's promoted into
//!    `confirmed` the same as an unambiguous cast would be.
//!
//!    Concretely: a Necromancer/Shadow-Knight character who casts
//!    `Lifedraw` (both classes, ambiguous on its own) constantly but only
//!    rarely lands a class-exclusive spell for either one stays stuck at
//!    two confirmed classes under rule 1 alone in most visits. A second
//!    ambiguous pool that shares only one class with `Lifedraw`'s -- say
//!    `Ward of Calliav` (Beastlord/Magician/Necromancer) -- intersects
//!    down to Necromancer alone. Checked against a real 49k-cast log
//!    before landing this: a pool that overlaps an already-confirmed class
//!    (`Root`: Cleric/Enchanter/Necromancer/Paladin/Shaman/Wizard, say)
//!    contributes nothing -- it's already fully explained by the confirmed
//!    class, so it's filtered out above before reaching this step, same as
//!    plain reinforcement. Only a pool that shares *no* class with what's
//!    already confirmed is real evidence about the open slot, and for this
//!    specific character that narrowed the pool of useful corroborating
//!    spells down to a handful of rare ones (`Ward of Calliav`, 4 casts in
//!    the whole log) rather than the common ones first assumed -- worth
//!    having, not a complete fix for a character whose ambiguous evidence
//!    is this lopsided.
//!
//!    Deliberately restricted to the *exactly-one-slot-open* case, not
//!    "narrow as far as possible from zero slots confirmed": with two or
//!    more slots open, a candidate pool from one ambiguous cast could
//!    belong to *either* open slot, so intersecting two such pools proves
//!    nothing and would risk manufacturing a false narrowing. One open
//!    slot is the only case where the logic is airtight.
//!
//! ## Partial evidence is not a smaller configuration
//!
//! A visit can end with fewer than `CLASS_COUNT` classes confirmed (lag,
//! or an unresolved elimination). That's real, but it must never be
//! *displayed* as if it were its own legitimate 1- or 2-class
//! configuration -- above level 10 no such thing exists. See
//! `visits_by_resolved_configuration`'s doc for how a partial gets folded
//! into the one full configuration it's an unambiguous subset of, or
//! reported as honestly unresolved when it's consistent with more than
//! one (or none yet).
//!
//! Membership isn't limited to spells: stances (`eqlp-app`'s `stancedata`,
//! `packs/stance_classes.json`) feed `observe_cast` the identical way a
//! spell does -- same elimination, same threshold, just a second source of
//! candidate pools. Still no combat-discipline or poison data (no verified
//! ability -> class mapping exists for either category; contributes zero
//! evidence, not wrong evidence), and no in-log signal exists at all for
//! some real class-restricted skills (Tracking opens a client-side window,
//! never a chat line -- confirmed against eqlwiki, not assumed).
//!
//! Evidence within one visit doesn't have to arrive in a convenient order:
//! an ambiguous cast seen *before* a visit's 2nd class is independently
//! confirmed is buffered, not discarded, and replayed the moment that 2nd
//! class does land -- see `VisitState::pending_pools`. One honest source
//! of imprecision remains, not hidden: **lag**. A class isn't confirmed
//! for a visit until *some* real evidence for it fires during that visit
//! at all -- a visit that ends before any does under-reports itself, and
//! nothing invents evidence that was never actually seen.

use std::collections::{BTreeSet, HashMap};

/// Confirmed game rule: every character above level 10 plays exactly this
/// many classes at once. Doubles as the exact threshold elimination waits
/// for (`CLASS_COUNT - 1` confirmed means exactly one slot open) -- see
/// this module's doc. Not otherwise enforced as a hard cap: a visit's
/// confirmed set is whatever the evidence actually showed, and forcing it
/// to `CLASS_COUNT` early would fabricate a class with no evidence.
pub const CLASS_COUNT: usize = 3;

/// Which zone visit a cast happened during, or `None` for "before any
/// `zone.enter` line was seen" -- an opaque index from the caller
/// (`eqlp_session::context::Spans::index_at` in practice), not interpreted
/// here beyond using it as a grouping key.
pub type ZoneVisit = Option<usize>;

#[derive(Debug, Default)]
struct VisitState {
    /// Classes confirmed for this visit, by either path in this module's
    /// doc. No record is kept of which path confirmed which class --
    /// both are equally real conclusions from actual evidence, never a
    /// guess, so there's nothing gained by distinguishing them downstream.
    confirmed: BTreeSet<String>,
    /// Running intersection of ambiguous-cast candidate pools since
    /// `confirmed` last reached exactly `CLASS_COUNT - 1` members -- see
    /// this module's doc for why elimination only ever runs with exactly
    /// one slot open. `None` until the first eligible ambiguous cast of
    /// the current one-slot-open window.
    narrowing: Option<BTreeSet<String>>,
    /// Ambiguous pools seen *before* this visit ever reached two confirmed
    /// classes -- real evidence, just too early to use yet (elimination
    /// only means anything once exactly one slot is open). Kept rather
    /// than dropped: a visit's own casts have no reason to happen in "the
    /// convenient order", and evidence that arrives seconds before the
    /// 2nd class confirms is exactly as real as evidence that arrives
    /// seconds after. Replayed once the visit reaches that point (see
    /// `reconcile_pending`), then cleared either way.
    pending_pools: Vec<BTreeSet<String>>,
}

#[derive(Debug, Default)]
struct EntityState {
    by_visit: HashMap<ZoneVisit, VisitState>,
    /// Class -> distinct visits with an unambiguous sighting, still under
    /// `MIN_UNAMBIGUOUS_CASTS` -- see `observe_cast`'s doc. Once a class
    /// crosses the threshold every pending visit is retroactively
    /// confirmed, not just the one that tipped it over, so a real class's
    /// own first sighting isn't punished for happening to be first; the
    /// entry is then removed and the class moves to `proven`.
    pending_unambiguous: HashMap<String, BTreeSet<ZoneVisit>>,
    /// Classes that have crossed the threshold -- confirmed outright from
    /// here on, no bookkeeping needed.
    proven: BTreeSet<String>,
}

/// An unambiguous cast only confirms a class once it's been seen on at
/// least this many distinct zone visits for the entity, ever (not just
/// this many casts -- a burst of the same spell within one visit is still
/// only one occasion) -- see `observe_cast`'s doc for the real case this
/// exists to catch: a single vendor-sold, off-class spell (real, wiki-
/// exclusive to one class, but bought and cast exactly once, never
/// repeated) otherwise reads as permanent proof of a class the player was
/// never actually playing. Confirmed against a real log: every *genuine*
/// class this player ever played showed evidence on multiple separate
/// visits; the one fluke showed on exactly one.
const MIN_UNAMBIGUOUS_CASTS: usize = 2;

/// Per-entity class evidence, grouped by zone visit. Never reset, never
/// decayed, nothing ever evicted -- see the module doc.
#[derive(Debug, Default)]
pub struct Detector {
    by_entity: HashMap<u32, EntityState>,
}

impl Detector {
    /// Record one cast as evidence for `entity` during `zone_visit`.
    /// `classes` is whatever the spell/class lookup says for this spell's
    /// rank-stripped base name -- empty if the spell isn't in the lookup.
    /// An empty slice contributes no evidence either way.
    ///
    /// An unambiguous cast (`classes.len() == 1`) needs `MIN_UNAMBIGUOUS_
    /// CASTS` distinct visits' worth of evidence for its class, entity-
    /// wide, before it confirms anything -- see that constant's doc.
    /// Below the threshold the visit is remembered, not confirmed yet; the
    /// visit that crosses the threshold retroactively confirms every
    /// pending visit along with itself.
    pub fn observe_cast(&mut self, entity: u32, zone_visit: ZoneVisit, classes: &[String]) {
        if classes.is_empty() {
            return;
        }
        let state = self.by_entity.entry(entity).or_default();
        if classes.len() == 1 {
            let class = &classes[0];
            if state.proven.contains(class) {
                let visit = state.by_visit.entry(zone_visit).or_default();
                visit.confirmed.insert(class.clone());
                Self::reconcile_pending(visit);
                return;
            }
            let pending = state.pending_unambiguous.entry(class.clone()).or_default();
            pending.insert(zone_visit);
            if pending.len() >= MIN_UNAMBIGUOUS_CASTS {
                let visits = state.pending_unambiguous.remove(class).unwrap();
                state.proven.insert(class.clone());
                for v in visits {
                    let visit = state.by_visit.entry(v).or_default();
                    visit.confirmed.insert(class.clone());
                    Self::reconcile_pending(visit);
                }
            }
            return;
        }
        let visit = state.by_visit.entry(zone_visit).or_default();
        // Ambiguous. If one of the candidates is already confirmed this
        // visit, this cast reinforces it -- nothing to change, that class
        // is already in `confirmed`. Otherwise it's a candidate for
        // elimination: see this module's doc for why that only ever
        // proceeds with exactly one slot still open.
        if classes.iter().any(|c| visit.confirmed.contains(c)) {
            return;
        }
        let candidates: BTreeSet<String> = classes.iter().cloned().collect();
        if visit.confirmed.len() != CLASS_COUNT - 1 {
            // Too early to narrow anything yet -- a visit's own casts have
            // no reason to land in "the convenient order". Kept, not
            // dropped: see `pending_pools`'s own doc.
            visit.pending_pools.push(candidates);
            return;
        }
        Self::apply_pool(visit, candidates);
    }

    /// One ambiguous candidate pool, intersected against whatever's
    /// already narrowing this visit's still-open slot. Shared by the live
    /// path (`observe_cast`, once exactly one slot is already open) and
    /// `reconcile_pending` (pools that arrived too early to apply live).
    fn apply_pool(visit: &mut VisitState, candidates: BTreeSet<String>) {
        let narrowed: BTreeSet<String> = match &visit.narrowing {
            Some(prev) => prev.intersection(&candidates).cloned().collect(),
            None => candidates.clone(),
        };
        if narrowed.is_empty() {
            // Two ambiguous casts whose candidate pools share no class in
            // common, while both genuinely describing the same single
            // open slot, is a contradiction real data shouldn't produce --
            // most likely one of the spells involved has a wrong entry in
            // `packs/spell_classes.json`. Restart narrowing from this
            // cast's own pool rather than staying stuck on an impossible
            // empty set for the rest of the visit.
            visit.narrowing = Some(candidates);
            return;
        }
        if narrowed.len() == 1 {
            visit.confirmed.extend(narrowed);
            visit.narrowing = None;
        } else {
            visit.narrowing = Some(narrowed);
        }
    }

    /// Once a visit reaches exactly `CLASS_COUNT - 1` confirmed classes,
    /// replays every ambiguous pool that arrived too early to use live
    /// (`pending_pools`) through the same elimination `apply_pool` already
    /// runs for pools that arrive on time -- in the order they were
    /// originally seen, so a real narrowing sequence still narrows the
    /// same way it would have if the 2nd class had simply confirmed a few
    /// casts sooner. No-op (and cheap) once there's nothing pending, so
    /// every insertion point into `confirmed` can call this unconditionally
    /// rather than each needing its own "is this now the 2nd class"
    /// bookkeeping.
    fn reconcile_pending(visit: &mut VisitState) {
        if visit.confirmed.len() != CLASS_COUNT - 1 || visit.pending_pools.is_empty() {
            return;
        }
        for candidates in std::mem::take(&mut visit.pending_pools) {
            if visit.confirmed.len() == CLASS_COUNT {
                break; // fully resolved by an earlier pool in this same replay
            }
            if candidates.iter().any(|c| visit.confirmed.contains(c)) {
                continue; // explained by something that confirmed after all
            }
            Self::apply_pool(visit, candidates);
        }
    }

    /// Every distinct class configuration this entity has confirmed across
    /// all its zone visits, most zone visits first (ties broken
    /// alphabetically by class list, for a stable order). A visit that
    /// confirmed nothing (no unambiguous cast landed, and elimination
    /// never reached a single candidate) contributes no configuration --
    /// not an empty-set entry.
    pub fn configurations_of(&self, entity: u32) -> Vec<(Vec<String>, usize)> {
        let Some(state) = self.by_entity.get(&entity) else {
            return Vec::new();
        };
        let mut counts: HashMap<Vec<String>, usize> = HashMap::new();
        for visit in state.by_visit.values() {
            if visit.confirmed.is_empty() {
                continue;
            }
            *counts
                .entry(visit.confirmed.iter().cloned().collect())
                .or_insert(0) += 1;
        }
        let mut v: Vec<(Vec<String>, usize)> = counts.into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        v
    }

    /// `configurations_of`, reconciled against the game's fixed-cardinality
    /// rule: a real configuration is always exactly `CLASS_COUNT` classes,
    /// so a visit that only ever confirmed fewer than that isn't a
    /// *different*, smaller configuration of its own -- above level 10
    /// there's no such thing -- it's incomplete evidence of one of the
    /// real ones. A partial configuration is folded into the one
    /// `CLASS_COUNT`-length configuration it's a subset of, when there's
    /// exactly one candidate (`{Enchanter, Magician}` can only be
    /// incomplete evidence of a config that contains both, so if only one
    /// confirmed config does, that's it); a partial consistent with more
    /// than one full configuration -- or with none confirmed yet at all --
    /// can't be resolved without guessing which one it really was, so its
    /// visits come back in the second element instead, honestly
    /// unresolved rather than picked arbitrarily or left looking like a
    /// real 1- or 2-class configuration.
    ///
    /// Returns visit membership, not just counts, so a caller can drill
    /// from "this configuration" down to the specific zone visits (and
    /// from there, encounters) that make it up -- counts alone can't
    /// answer "which ones".
    pub fn visits_by_resolved_configuration(
        &self,
        entity: u32,
    ) -> (Vec<(Vec<String>, Vec<ZoneVisit>)>, Vec<ZoneVisit>) {
        let Some(state) = self.by_entity.get(&entity) else {
            return (Vec::new(), Vec::new());
        };
        let mut by_raw: HashMap<Vec<String>, Vec<ZoneVisit>> = HashMap::new();
        for (&visit, vs) in &state.by_visit {
            if vs.confirmed.is_empty() {
                continue;
            }
            by_raw
                .entry(vs.confirmed.iter().cloned().collect())
                .or_default()
                .push(visit);
        }

        let mut full: Vec<(BTreeSet<String>, Vec<ZoneVisit>)> = by_raw
            .iter()
            .filter(|(c, _)| c.len() == CLASS_COUNT)
            .map(|(c, vs)| (c.iter().cloned().collect(), vs.clone()))
            .collect();

        let mut unresolved: Vec<ZoneVisit> = Vec::new();
        for (c, vs) in by_raw.iter().filter(|(c, _)| c.len() < CLASS_COUNT) {
            let partial: BTreeSet<String> = c.iter().cloned().collect();
            let matches: Vec<usize> = full
                .iter()
                .enumerate()
                .filter(|(_, (f, _))| partial.is_subset(f))
                .map(|(i, _)| i)
                .collect();
            match matches.as_slice() {
                [i] => full[*i].1.extend(vs.iter().copied()),
                _ => unresolved.extend(vs.iter().copied()),
            }
        }

        let mut out: Vec<(Vec<String>, Vec<ZoneVisit>)> = full
            .into_iter()
            .map(|(c, vs)| (c.into_iter().collect(), vs))
            .collect();
        out.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then_with(|| a.0.cmp(&b.0)));
        (out, unresolved)
    }

    /// The classes confirmed for `entity` during exactly `zone_visit`, as
    /// of however much evidence has accumulated for that visit so far.
    /// For tagging one specific fight with the configuration active around
    /// it (`Ingest::record_history` in `eqlp-app`) -- since evidence
    /// accumulates in strict log-time order, querying this at the moment a
    /// fight in that same visit closes gives an honest "as of this fight"
    /// answer, not a lifetime or cross-visit blend.
    pub fn configuration_of_visit(&self, entity: u32, zone_visit: ZoneVisit) -> Vec<String> {
        self.by_entity
            .get(&entity)
            .and_then(|s| s.by_visit.get(&zone_visit))
            .map(|v| v.confirmed.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Every entity with any evidence at all, ever.
    pub fn known_entities(&self) -> impl Iterator<Item = u32> + '_ {
        self.by_entity.keys().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn one_sighting_of_an_unambiguous_class_is_not_enough() {
        let mut d = Detector::default();
        d.observe_cast(1, Some(0), &strs(&["Wizard"]));
        assert!(d.configuration_of_visit(1, Some(0)).is_empty());
    }

    #[test]
    fn two_distinct_visits_confirm_an_unambiguous_class() {
        let mut d = Detector::default();
        d.observe_cast(1, Some(0), &strs(&["Wizard"]));
        d.observe_cast(1, Some(1), &strs(&["Wizard"]));
        // Retroactive: both visits, not just the one that tipped it over.
        assert_eq!(d.configuration_of_visit(1, Some(0)), strs(&["Wizard"]));
        assert_eq!(d.configuration_of_visit(1, Some(1)), strs(&["Wizard"]));
    }

    #[test]
    fn a_burst_within_one_visit_is_only_one_occasion() {
        let mut d = Detector::default();
        for _ in 0..5 {
            d.observe_cast(1, Some(0), &strs(&["Wizard"]));
        }
        assert!(
            d.configuration_of_visit(1, Some(0)).is_empty(),
            "5 casts on the same visit must not substitute for 2 distinct visits"
        );
    }

    #[test]
    fn elimination_narrows_when_the_ambiguous_cast_arrives_after_two_slots_are_confirmed() {
        let mut d = Detector::default();
        // Confirm Enchanter and Wizard first (proven, 2 distinct visits each).
        for v in [0, 1] {
            d.observe_cast(1, Some(v), &strs(&["Enchanter"]));
            d.observe_cast(1, Some(v), &strs(&["Wizard"]));
        }
        // "Proven" globally, but confirmation is still per-visit -- each
        // visit re-earns it with its own cast, same as the real Faydark
        // case this module was built to actually resolve.
        d.observe_cast(1, Some(2), &strs(&["Enchanter"]));
        d.observe_cast(1, Some(2), &strs(&["Wizard"]));
        // Now, on this same visit, two ambiguous casts whose pools
        // intersect to exactly Cleric narrow the open slot outright.
        d.observe_cast(1, Some(2), &strs(&["Beastlord", "Cleric", "Druid"]));
        d.observe_cast(1, Some(2), &strs(&["Cleric", "Paladin", "Shaman"]));
        let cfg = d.configuration_of_visit(1, Some(2));
        assert_eq!(cfg, strs(&["Cleric", "Enchanter", "Wizard"]));
    }

    /// The real fix: the exact same evidence, in the opposite order --
    /// ambiguous casts land *before* the visit's 2nd class is confirmed.
    /// Must resolve identically to the test above, not lose the early
    /// evidence just because it arrived first.
    #[test]
    fn elimination_still_narrows_when_the_ambiguous_casts_arrive_before_two_slots_are_confirmed() {
        let mut d = Detector::default();
        for v in [0, 1] {
            d.observe_cast(1, Some(v), &strs(&["Enchanter"]));
            d.observe_cast(1, Some(v), &strs(&["Wizard"]));
        }
        // A fresh visit: the ambiguous evidence lands first this time,
        // before either Enchanter or Wizard has actually recast this visit.
        d.observe_cast(1, Some(2), &strs(&["Beastlord", "Cleric", "Druid"]));
        d.observe_cast(1, Some(2), &strs(&["Cleric", "Paladin", "Shaman"]));
        assert!(
            d.configuration_of_visit(1, Some(2)).is_empty(),
            "not yet -- neither slot is confirmed on this visit at all so far"
        );
        // Now the visit's own Enchanter/Wizard casts land, reaching 2/3 --
        // the buffered ambiguous evidence above should retroactively apply.
        d.observe_cast(1, Some(2), &strs(&["Enchanter"]));
        d.observe_cast(1, Some(2), &strs(&["Wizard"]));
        let cfg = d.configuration_of_visit(1, Some(2));
        assert_eq!(cfg, strs(&["Cleric", "Enchanter", "Wizard"]));
    }

    #[test]
    fn a_pool_sharing_no_class_with_the_running_intersection_restarts_instead_of_sticking_empty() {
        let mut d = Detector::default();
        for v in [0, 1] {
            d.observe_cast(1, Some(v), &strs(&["Enchanter"]));
            d.observe_cast(1, Some(v), &strs(&["Wizard"]));
        }
        d.observe_cast(1, Some(2), &strs(&["Enchanter"]));
        d.observe_cast(1, Some(2), &strs(&["Wizard"]));
        // Two ambiguous pools sharing nothing -- a contradiction (most
        // likely bad data upstream); narrowing restarts from the second
        // pool rather than staying stuck on an impossible empty set.
        d.observe_cast(1, Some(2), &strs(&["Beastlord", "Druid"]));
        d.observe_cast(1, Some(2), &strs(&["Paladin", "Shadow Knight"])); // shares nothing with the above
        assert_eq!(
            d.configuration_of_visit(1, Some(2)),
            strs(&["Enchanter", "Wizard"]),
            "3rd slot still open -- the contradiction must not fabricate a class"
        );
        d.observe_cast(1, Some(2), &strs(&["Paladin", "Warrior"]));
        let cfg = d.configuration_of_visit(1, Some(2));
        assert_eq!(cfg, strs(&["Enchanter", "Paladin", "Wizard"]));
    }

    #[test]
    fn an_ambiguous_pool_that_already_overlaps_a_confirmed_class_is_just_reinforcement() {
        let mut d = Detector::default();
        for v in [0, 1] {
            d.observe_cast(1, Some(v), &strs(&["Enchanter"]));
        }
        // Ambiguous, but includes the already-confirmed Enchanter -- no
        // new open-slot evidence, must not disturb anything.
        d.observe_cast(1, Some(0), &strs(&["Enchanter", "Magician"]));
        assert_eq!(d.configuration_of_visit(1, Some(0)), strs(&["Enchanter"]));
    }
}
