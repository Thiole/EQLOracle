//! why: infers class configuration(s) from casts -- no log line states it
//!
//! Game rule: exactly CLASS_COUNT (3) classes at once, swappable in town.
//!
//! ## Design: grouped per zone visit, nothing evicted
//! why: a single rolling value crowds out rare-but-real configurations --
//! grouping by zone visit (loadout is stable within one) keeps every
//! distinct configuration this entity has ever played, not just the loudest.
//!
//! ## Confirming a class: unambiguous evidence, or elimination
//! why: an unambiguous cast confirms after MIN_UNAMBIGUOUS_CASTS repeats
//! (one-off off-class spells don't count); once CLASS_COUNT-1 classes are
//! confirmed, an ambiguous cast whose candidates exclude them narrows the
//! provably-open last slot by intersection -- only valid with one slot open.
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
    /// why: crossed the threshold, confirmed outright from here on
    proven: BTreeSet<String>,
}

/// why: needs 2+ distinct visits -- a one-off vendor spell shouldn't confirm
const MIN_UNAMBIGUOUS_CASTS: usize = 2;

/// why: per-entity evidence by zone visit, never reset/decayed/evicted
#[derive(Debug, Default)]
pub struct Detector {
    by_entity: HashMap<u32, EntityState>,
}

impl Detector {
    /// why: unambiguous needs MIN_UNAMBIGUOUS_CASTS visits to confirm
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
        // why: reinforces if already confirmed, else a candidate for elimination
        if classes.iter().any(|c| visit.confirmed.contains(c)) {
            return;
        }
        let candidates: BTreeSet<String> = classes.iter().cloned().collect();
        if visit.confirmed.len() != CLASS_COUNT - 1 {
            // why: too early to narrow -- kept, not dropped, see pending_pools
            visit.pending_pools.push(candidates);
            return;
        }
        Self::apply_pool(visit, candidates);
    }

    /// why: intersects one pool into the running narrowing, live or replayed
    fn apply_pool(visit: &mut VisitState, candidates: BTreeSet<String>) {
        let narrowed: BTreeSet<String> = match &visit.narrowing {
            Some(prev) => prev.intersection(&candidates).cloned().collect(),
            None => candidates.clone(),
        };
        if narrowed.is_empty() {
            // why: empty intersection means bad data -- restart from this pool
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

    /// why: replays pending_pools in order once the 2nd class confirms
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

    #[test]
    fn elimination_narrows_when_the_ambiguous_cast_arrives_after_two_slots_are_confirmed() {
        let mut d = Detector::default();
        // why: proven, 2 distinct visits each
        for v in [0, 1] {
            d.observe_cast(1, Some(v), &strs(&["Enchanter"]));
            d.observe_cast(1, Some(v), &strs(&["Wizard"]));
        }
        // why: proven is global, but confirmation is still re-earned per-visit
        d.observe_cast(1, Some(2), &strs(&["Enchanter"]));
        d.observe_cast(1, Some(2), &strs(&["Wizard"]));
        // why: two pools intersecting to exactly Cleric narrow it outright
        d.observe_cast(1, Some(2), &strs(&["Beastlord", "Cleric", "Druid"]));
        d.observe_cast(1, Some(2), &strs(&["Cleric", "Paladin", "Shaman"]));
        let cfg = d.configuration_of_visit(1, Some(2));
        assert_eq!(cfg, strs(&["Cleric", "Enchanter", "Wizard"]));
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
        // why: 2nd class confirms, buffered evidence should apply retroactively
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
        // why: no-overlap pools = bad data, restart narrowing instead of sticking
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
        // why: includes an already-confirmed class, must not disturb anything
        d.observe_cast(1, Some(0), &strs(&["Enchanter", "Magician"]));
        assert_eq!(d.configuration_of_visit(1, Some(0)), strs(&["Enchanter"]));
    }
}
