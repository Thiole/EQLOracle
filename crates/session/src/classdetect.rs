//! why: infers class configuration(s) from casts -- no log line states it
//!
//! Game rule: exactly CLASS_COUNT (3) classes at once, swappable in town.
//!
//! ## Design: grouped per zone visit, nothing evicted
//! why: a single rolling value crowds out rare-but-real configurations --
//! grouping by zone visit (loadout is stable within one) keeps every
//! distinct configuration this entity has ever played, not just the loudest.
//!
//! ## Confirming a class: unambiguous evidence, or elimination -- as two
//! ## separate evidence kinds, not one
//! why: real bug, caught live against a real 2nd player's log. A
//! handful of real invocations/stances span 6-12 of the 15 classes each
//! (Recovery/Over Channel/Inversion are all the *same* 12-class list;
//! Divine is 6; Spellblade is 4). A player who casts several of these
//! often enough can have two such pools intersect down to a single
//! residual class *by pure combinatorial chance*, entirely within one
//! zone visit -- the old code trusted that instantly, same confidence
//! as a real unambiguous cast repeated on 2 distinct visits.
//!
//! Confirmed wrong live, then traced to ground truth: a player who has
//! "never played Beastlord" (their own words) had it show up in two
//! resolved configurations, with *zero* Beastlord-eligible spell ever
//! narrower than a 2-class pool ever landing -- purely elimination
//! coincidence. Separately, that same player's data *did* show
//! Enchanter repeatedly, but that one turned out to be real: genuinely
//! Enchanter-exclusive spells (Mesmerize, several rank-appropriate
//! Illusion spells, one bought from an Enchanter-only vendor per its
//! own `where_to_obtain`) landed dozens of times each. Elimination
//! coincidence and real unambiguous spell evidence look identical once
//! narrowed to one candidate -- but they are not equally trustworthy,
//! so they're no longer treated as interchangeable:
//!
//! - An unambiguous cast still needs `MIN_UNAMBIGUOUS_CASTS` (2)
//!   distinct visits, tracked in `pending_unambiguous`.
//! - Elimination narrowing to exactly one candidate needs
//!   `MIN_ELIMINATION_CASTS` (3) distinct visits, tracked separately in
//!   `pending_elimination` -- confirmed live that even a 2nd
//!   independent coincidence isn't rare enough to trust on its own; see
//!   `MIN_ELIMINATION_CASTS`'s own doc.
//!
//! Both still fully retroactive once their own bar is crossed (every
//! visit that pointed at the class gets it, not just the one that
//! tipped it over), and once *either* threshold proves a class, it's
//! `proven` outright from then on regardless of which kind of evidence
//! crosses it -- see `propose`.
//!
//! ## Partial evidence is not a smaller configuration
//! why: a below-CLASS_COUNT visit is lag/unresolved, never shown as its
//! own legitimate config -- see `visits_by_resolved_configuration`.
//! Stances feed the same pipeline as spells; disciplines/poisons/Tracking
//! have no verified class-mapping and contribute no evidence either way.

use std::collections::{BTreeSet, HashMap};

/// why: fixed game rule, also the elimination threshold -- never forced early
pub const CLASS_COUNT: usize = 3;

/// why: opaque grouping key, `None` for before the first zone.enter
pub type ZoneVisit = Option<usize>;

/// why: named to avoid tripping clippy's type_complexity on a bare tuple
pub type ConfiguredVisits = Vec<(Vec<String>, Vec<ZoneVisit>)>;

#[derive(Debug, Default)]
struct VisitState {
    /// why: confirmed by either path, undistinguished -- both are real
    confirmed: BTreeSet<String>,
    /// why: intersection since CLASS_COUNT-1 confirmed, None until then
    narrowing: Option<BTreeSet<String>>,
    /// why: ambiguous pools too early to use, replayed once the 2nd class lands
    pending_pools: Vec<BTreeSet<String>>,
}

#[derive(Debug, Default)]
struct EntityState {
    by_visit: HashMap<ZoneVisit, VisitState>,
    /// why: visits with an unambiguous sighting, still under the threshold
    pending_unambiguous: HashMap<String, BTreeSet<ZoneVisit>>,
    /// why: visits where elimination narrowed to this class, still under
    /// its own (stricter) threshold -- kept separate from
    /// pending_unambiguous rather than merged, so a class never gets
    /// proven by mixing one real spell sighting with one lucky
    /// intersection; each evidence kind has to clear its own bar on its own.
    pending_elimination: HashMap<String, BTreeSet<ZoneVisit>>,
    /// why: crossed either threshold, confirmed outright from here on
    proven: BTreeSet<String>,
}

/// why: needs 2+ distinct visits -- a one-off vendor spell shouldn't confirm
const MIN_UNAMBIGUOUS_CASTS: usize = 2;

/// why: real bug, caught live -- 2 distinct visits was enough for
/// *unambiguous* evidence (a genuinely class-exclusive spell landing
/// twice), but elimination narrowing is a much weaker signal (several
/// real invocations/stances span 6-12 of 15 classes; two such pools can
/// intersect to a single residual class by pure combinatorial chance).
/// Confirmed live: a real player's data had elimination alone narrow to
/// the same wrong class on 2 separate visits (161, 179) with zero
/// direct spell evidence ever supporting it. A 3rd independent
/// coincidence landing on the same wrong class is meaningfully less
/// likely than a 2nd, without raising the bar for genuinely unambiguous
/// spell evidence at all.
const MIN_ELIMINATION_CASTS: usize = 3;

/// why: per-entity evidence by zone visit, never reset/decayed/evicted
#[derive(Debug, Default)]
pub struct Detector {
    by_entity: HashMap<u32, EntityState>,
}

/// why: which pending map / threshold a proposed class goes through --
/// see MIN_ELIMINATION_CASTS's own doc for why they differ
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Evidence {
    Unambiguous,
    Elimination,
}

impl Detector {
    /// why: unambiguous needs MIN_UNAMBIGUOUS_CASTS visits to confirm; an
    /// ambiguous pool that narrows to exactly one candidate is proposed
    /// through the identical gate rather than confirmed on the spot --
    /// see this module's own doc for why single-visit narrowing alone
    /// isn't trustworthy.
    pub fn observe_cast(&mut self, entity: u32, zone_visit: ZoneVisit, classes: &[String]) {
        if classes.is_empty() {
            return;
        }
        let state = self.by_entity.entry(entity).or_default();
        if classes.len() == 1 {
            Self::propose(state, zone_visit, classes[0].clone(), Evidence::Unambiguous);
            return;
        }
        let candidates: BTreeSet<String> = classes.iter().cloned().collect();
        let narrowed = {
            let visit = state.by_visit.entry(zone_visit).or_default();
            // why: reinforces if already confirmed, else a candidate for elimination
            if candidates.iter().any(|c| visit.confirmed.contains(c)) {
                return;
            }
            if visit.confirmed.len() != CLASS_COUNT - 1 {
                // why: too early to narrow -- kept, not dropped, see pending_pools
                visit.pending_pools.push(candidates);
                return;
            }
            Self::narrow(visit, candidates)
        };
        if let Some(class) = narrowed {
            Self::propose(state, zone_visit, class, Evidence::Elimination);
        }
    }

    /// why: single funnel for both evidence kinds -- an unambiguous cast
    /// and a pool narrowed to exactly one candidate are both "this visit
    /// points at class X" claims, but not equally strong ones (see this
    /// module's own doc for why elimination gets its own, stricter
    /// threshold) -- each kind accumulates in its own pending map,
    /// never mixed, so a class is never proven by combining one real
    /// spell sighting with lucky intersections. Confirming a class can
    /// unblock that visit's own buffered pools (`reconcile_pending`),
    /// which can themselves narrow to a fresh single candidate -- always
    /// elimination evidence, queued rather than recursed so a chain of
    /// confirmations across many visits resolves without deep recursion.
    fn propose(state: &mut EntityState, zone_visit: ZoneVisit, class: String, kind: Evidence) {
        let mut queue = vec![(zone_visit, class, kind)];
        while let Some((zv, class, kind)) = queue.pop() {
            if state.proven.contains(&class) {
                let visit = state.by_visit.entry(zv).or_default();
                if visit.confirmed.insert(class) {
                    queue.extend(
                        Self::reconcile_pending(visit)
                            .into_iter()
                            .map(|c| (zv, c, Evidence::Elimination)),
                    );
                }
                continue;
            }
            let (pending_map, threshold) = match kind {
                Evidence::Unambiguous => (&mut state.pending_unambiguous, MIN_UNAMBIGUOUS_CASTS),
                Evidence::Elimination => (&mut state.pending_elimination, MIN_ELIMINATION_CASTS),
            };
            let pending = pending_map.entry(class.clone()).or_default();
            pending.insert(zv);
            if pending.len() < threshold {
                continue;
            }
            let visits = pending_map.remove(&class).unwrap();
            // why: whichever kind crossed its own bar first proves it --
            // the other kind's now-stale partial evidence (if any) is
            // harmless leftover, but dropped for tidiness
            state.pending_unambiguous.remove(&class);
            state.pending_elimination.remove(&class);
            state.proven.insert(class.clone());
            for v in visits {
                let visit = state.by_visit.entry(v).or_default();
                if visit.confirmed.insert(class.clone()) {
                    queue.extend(
                        Self::reconcile_pending(visit)
                            .into_iter()
                            .map(|c| (v, c, Evidence::Elimination)),
                    );
                }
            }
        }
    }

    /// why: intersects one pool into the running narrowing, live or
    /// replayed -- returns the single remaining candidate whenever
    /// narrowing sits at exactly one, for the caller to route through
    /// `propose`'s own cross-visit corroboration (never writes
    /// `confirmed` directly -- that's `propose`'s job once corroborated).
    fn narrow(visit: &mut VisitState, candidates: BTreeSet<String>) -> Option<String> {
        let narrowed: BTreeSet<String> = match &visit.narrowing {
            Some(prev) => prev.intersection(&candidates).cloned().collect(),
            None => candidates.clone(),
        };
        if narrowed.is_empty() {
            // why: empty intersection means bad data -- restart from this pool
            visit.narrowing = Some(candidates);
            return None;
        }
        let single = (narrowed.len() == 1)
            .then(|| narrowed.iter().next().cloned())
            .flatten();
        visit.narrowing = Some(narrowed);
        single
    }

    /// why: replays pending_pools in order once the 2nd class confirms;
    /// returns any classes freshly narrowed to one by that replay, for
    /// `propose`'s own queue -- doesn't confirm them itself
    fn reconcile_pending(visit: &mut VisitState) -> Vec<String> {
        if visit.confirmed.len() != CLASS_COUNT - 1 || visit.pending_pools.is_empty() {
            return Vec::new();
        }
        let mut narrowed_classes = Vec::new();
        for candidates in std::mem::take(&mut visit.pending_pools) {
            if visit.confirmed.len() == CLASS_COUNT {
                break; // fully resolved by an earlier pool in this same replay
            }
            if candidates.iter().any(|c| visit.confirmed.contains(c)) {
                continue; // explained by something that confirmed after all
            }
            if let Some(class) = Self::narrow(visit, candidates) {
                narrowed_classes.push(class);
            }
        }
        narrowed_classes
    }

    /// why: every distinct configuration, most-visits-first, empty ones excluded
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

    /// why: folds partials into their one matching full config, or leaves
    /// genuinely-ambiguous ones unresolved rather than guessing which
    pub fn visits_by_resolved_configuration(
        &self,
        entity: u32,
    ) -> (ConfiguredVisits, Vec<ZoneVisit>) {
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

    /// why: confirmed classes for one visit, honest "as of this fight" answer
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

    /// why: real bug, caught live -- a single visit's own elimination
    /// narrowing used to confirm outright. Now it's proposed evidence
    /// like any other, sitting at 2 classes until a 2nd distinct visit
    /// independently narrows to the same one.
    #[test]
    fn a_single_visits_own_elimination_narrowing_is_not_enough_by_itself() {
        let mut d = Detector::default();
        for v in [0, 1] {
            d.observe_cast(1, Some(v), &strs(&["Enchanter"]));
            d.observe_cast(1, Some(v), &strs(&["Wizard"]));
        }
        d.observe_cast(1, Some(2), &strs(&["Enchanter"]));
        d.observe_cast(1, Some(2), &strs(&["Wizard"]));
        // why: two pools intersecting to exactly Cleric, all on visit 2 alone
        d.observe_cast(1, Some(2), &strs(&["Beastlord", "Cleric", "Druid"]));
        d.observe_cast(1, Some(2), &strs(&["Cleric", "Paladin", "Shaman"]));
        assert_eq!(
            d.configuration_of_visit(1, Some(2)),
            strs(&["Enchanter", "Wizard"]),
            "narrowed to Cleric on just this one visit -- not proof by itself"
        );
        // why: repeating the exact same two pools on the same visit must
        // not substitute for a 2nd *distinct* visit either
        for _ in 0..5 {
            d.observe_cast(1, Some(2), &strs(&["Beastlord", "Cleric", "Druid"]));
            d.observe_cast(1, Some(2), &strs(&["Cleric", "Paladin", "Shaman"]));
        }
        assert_eq!(
            d.configuration_of_visit(1, Some(2)),
            strs(&["Enchanter", "Wizard"]),
            "still just one visit's worth of evidence, however many times it repeats"
        );
    }

    #[test]
    fn elimination_confirms_once_three_distinct_visits_narrow_to_the_same_class() {
        let mut d = Detector::default();
        for v in [0, 1] {
            d.observe_cast(1, Some(v), &strs(&["Enchanter"]));
            d.observe_cast(1, Some(v), &strs(&["Wizard"]));
        }
        for v in [2, 3, 4] {
            d.observe_cast(1, Some(v), &strs(&["Enchanter"]));
            d.observe_cast(1, Some(v), &strs(&["Wizard"]));
            d.observe_cast(1, Some(v), &strs(&["Beastlord", "Cleric", "Druid"]));
            d.observe_cast(1, Some(v), &strs(&["Cleric", "Paladin", "Shaman"]));
        }
        // why: retroactive, same as the pure-unambiguous path -- every
        // visit that pointed at Cleric gets it, not just the 3rd
        for v in [2, 3, 4] {
            assert_eq!(
                d.configuration_of_visit(1, Some(v)),
                strs(&["Cleric", "Enchanter", "Wizard"])
            );
        }
    }

    /// why: same evidence, opposite order -- must still resolve identically
    #[test]
    fn elimination_still_narrows_when_the_ambiguous_casts_arrive_before_two_slots_are_confirmed() {
        let mut d = Detector::default();
        for v in [0, 1] {
            d.observe_cast(1, Some(v), &strs(&["Enchanter"]));
            d.observe_cast(1, Some(v), &strs(&["Wizard"]));
        }
        // why: ambiguous evidence lands before this visit's own 2nd class
        d.observe_cast(1, Some(2), &strs(&["Beastlord", "Cleric", "Druid"]));
        d.observe_cast(1, Some(2), &strs(&["Cleric", "Paladin", "Shaman"]));
        assert!(
            d.configuration_of_visit(1, Some(2)).is_empty(),
            "not yet -- neither slot is confirmed on this visit at all so far"
        );
        // why: 2nd class confirms, buffered evidence narrows retroactively
        // -- but still just this one visit, not enough by itself
        d.observe_cast(1, Some(2), &strs(&["Enchanter"]));
        d.observe_cast(1, Some(2), &strs(&["Wizard"]));
        assert_eq!(
            d.configuration_of_visit(1, Some(2)),
            strs(&["Enchanter", "Wizard"])
        );
        // why: a 2nd distinct visit, same story -- still not enough (needs 3)
        d.observe_cast(1, Some(3), &strs(&["Beastlord", "Cleric", "Druid"]));
        d.observe_cast(1, Some(3), &strs(&["Cleric", "Paladin", "Shaman"]));
        d.observe_cast(1, Some(3), &strs(&["Enchanter"]));
        d.observe_cast(1, Some(3), &strs(&["Wizard"]));
        assert_eq!(
            d.configuration_of_visit(1, Some(2)),
            strs(&["Enchanter", "Wizard"]),
            "narrowed to Cleric on 2 visits now -- still not proof by itself"
        );
        // why: a 3rd distinct visit finally corroborates it
        d.observe_cast(1, Some(4), &strs(&["Beastlord", "Cleric", "Druid"]));
        d.observe_cast(1, Some(4), &strs(&["Cleric", "Paladin", "Shaman"]));
        d.observe_cast(1, Some(4), &strs(&["Enchanter"]));
        d.observe_cast(1, Some(4), &strs(&["Wizard"]));
        for v in [2, 3, 4] {
            assert_eq!(
                d.configuration_of_visit(1, Some(v)),
                strs(&["Cleric", "Enchanter", "Wizard"])
            );
        }
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
        // why: no-overlap pools = bad data, restart narrowing instead of sticking
        d.observe_cast(1, Some(2), &strs(&["Beastlord", "Druid"]));
        d.observe_cast(1, Some(2), &strs(&["Paladin", "Shadow Knight"])); // shares nothing with the above
        assert_eq!(
            d.configuration_of_visit(1, Some(2)),
            strs(&["Enchanter", "Wizard"]),
            "3rd slot still open -- the contradiction must not fabricate a class"
        );
        d.observe_cast(1, Some(2), &strs(&["Paladin", "Warrior"]));
        assert_eq!(
            d.configuration_of_visit(1, Some(2)),
            strs(&["Enchanter", "Wizard"]),
            "narrowed to Paladin, but still only this one visit's own evidence"
        );
        // why: a 2nd distinct visit repeats the same restart-then-narrow
        // result -- still not enough (needs 3 for elimination evidence)
        d.observe_cast(1, Some(3), &strs(&["Enchanter"]));
        d.observe_cast(1, Some(3), &strs(&["Wizard"]));
        d.observe_cast(1, Some(3), &strs(&["Beastlord", "Druid"]));
        d.observe_cast(1, Some(3), &strs(&["Paladin", "Shadow Knight"]));
        d.observe_cast(1, Some(3), &strs(&["Paladin", "Warrior"]));
        assert_eq!(
            d.configuration_of_visit(1, Some(2)),
            strs(&["Enchanter", "Wizard"]),
            "narrowed to Paladin on 2 visits now -- still not proof by itself"
        );
        // why: a 3rd distinct visit finally corroborates it
        d.observe_cast(1, Some(4), &strs(&["Enchanter"]));
        d.observe_cast(1, Some(4), &strs(&["Wizard"]));
        d.observe_cast(1, Some(4), &strs(&["Beastlord", "Druid"]));
        d.observe_cast(1, Some(4), &strs(&["Paladin", "Shadow Knight"]));
        d.observe_cast(1, Some(4), &strs(&["Paladin", "Warrior"]));
        let cfg = d.configuration_of_visit(1, Some(2));
        assert_eq!(cfg, strs(&["Enchanter", "Paladin", "Wizard"]));
    }

    /// why: direct regression test for the real incident -- Recovery/Over
    /// Channel/Inversion are the *same* real 12-class pool (everything
    /// but Berserker/Monk/Rogue/Warrior), cast often enough by a real
    /// player that two of them intersecting with a smaller unrelated
    /// pool coincidentally landed on Beastlord, on a player who has
    /// never played Beastlord. This reproduces that shape directly.
    #[test]
    fn broad_real_invocation_pools_never_confirm_a_class_from_one_visit_alone() {
        let mut d = Detector::default();
        let recovery = strs(&[
            "Bard",
            "Beastlord",
            "Cleric",
            "Druid",
            "Enchanter",
            "Magician",
            "Necromancer",
            "Paladin",
            "Ranger",
            "Shadow Knight",
            "Shaman",
            "Wizard",
        ]);
        let divine = strs(&[
            "Beastlord",
            "Cleric",
            "Druid",
            "Paladin",
            "Ranger",
            "Shaman",
        ]);
        let spellblade = strs(&["Beastlord", "Paladin", "Ranger", "Shadow Knight"]);

        for v in [0, 1] {
            d.observe_cast(1, Some(v), &strs(&["Bard"]));
            d.observe_cast(1, Some(v), &strs(&["Druid"]));
        }
        // why: Bard/Druid confirmed on visit 2, then 3 real broad
        // invocation pools narrow the field to {Beastlord, Paladin,
        // Ranger}, and one more small ambiguous pool isolates
        // Beastlord outright -- on this one visit alone, must not confirm it
        d.observe_cast(1, Some(2), &strs(&["Bard"]));
        d.observe_cast(1, Some(2), &strs(&["Druid"]));
        d.observe_cast(1, Some(2), &recovery);
        d.observe_cast(1, Some(2), &divine);
        d.observe_cast(1, Some(2), &spellblade);
        d.observe_cast(1, Some(2), &strs(&["Beastlord", "Cleric", "Magician"]));
        assert_eq!(
            d.configuration_of_visit(1, Some(2)),
            strs(&["Bard", "Druid"]),
            "Beastlord must not be confirmed from one visit's broad-pool coincidence"
        );
    }

    #[test]
    fn an_ambiguous_pool_that_already_overlaps_a_confirmed_class_is_just_reinforcement() {
        let mut d = Detector::default();
        for v in [0, 1] {
            d.observe_cast(1, Some(v), &strs(&["Enchanter"]));
        }
        // why: includes an already-confirmed class, must not disturb anything
        d.observe_cast(1, Some(0), &strs(&["Enchanter", "Magician"]));
        assert_eq!(d.configuration_of_visit(1, Some(0)), strs(&["Enchanter"]));
    }
}
