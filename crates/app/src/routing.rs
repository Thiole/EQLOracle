//! why: zone-to-zone route finding, weighted by real in-zone walking
//! distance (`pathfind::find_path`), not just hop count. See
//! `docs/design/maps.md`'s "Pathfinding" section for the full rationale.
//!
//! **Dijkstra over real distance, not "generate candidates then score."**
//! An earlier two-stage version fully real-pathfound every hop of every
//! candidate before comparing -- measured 25s for a trivial 1-hop query
//! because it also scored losing multi-hop alternates. Dijkstra explores
//! in increasing real-distance order and finalizes each zone the first
//! time it's popped -- an already-more-expensive partial route is never
//! extended further, the algorithm's own termination property, not a cap.
//!
//! Real per-hop distance is the expensive part, computed lazily (only
//! when Dijkstra settles a zone) and memoized process-lifetime via
//! `cached_hop_distance`, keyed on `(from_zone, from_position, to_zone)`
//! -- sound because every walk-hop's `from` is deterministic. Teleport
//! edges stay O(1), no geometry lookup at all.

use crate::mapsdata;
use crate::pathfind;
use crate::state::LockRecover;
use crate::teleportdata;
use crate::zonedata;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

/// why: avoids re-loading/re-parsing the same zone map per visit --
/// measured 12.7s for a 13-hop query before this cache existed. Keyed by
/// shortname only, process-lifetime. Arc, not clone-per-hit -- the
/// largest zones run 26k+ wall segments.
fn cached_map(base_dir: &Path, shortname: &str) -> Option<Arc<mapsdata::ParsedZoneMap>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Arc<mapsdata::ParsedZoneMap>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(m) = cache.lock_recover().get(shortname) {
        return Some(m.clone());
    }
    let map = Arc::new(mapsdata::load_zone_map(base_dir, None, shortname).ok()?);
    cache
        .lock_recover()
        .insert(shortname.to_string(), map.clone());
    Some(map)
}

/// why: small relative to real crossings (measured ~400-15,000 units),
/// not literally free. Note: gated per-query, but the graph itself
/// offers every teleport edge unconditionally -- `RouteHopDto` tags each
/// with its exact spell name so the player judges real viability.
const TELEPORT_HOP_COST: f64 = 200.0;

/// why: relocating to a zone's own succor point, reachable from anywhere
/// in the zone (Lesser Evacuate or a tier change), smaller than a full jump
const SUCCOR_WARP_COST: f64 = 100.0;

#[derive(Debug, Clone, PartialEq)]
pub enum HopKind {
    Walk,
    /// Cast this spell to make the jump.
    Teleport(String),
    /// why: explicit step for a winning succor relay -- used to be
    /// silently folded into the walk-hop number with no indication a
    /// real action was needed; arrives in the same zone the walk starts from
    Succor,
}

#[derive(Debug, Clone)]
pub struct RouteHop {
    /// The zone this hop *arrives in*.
    pub zone: String,
    pub kind: HopKind,
    pub distance: f64,
}

#[derive(Debug, Clone)]
pub struct ZoneRoute {
    pub hops: Vec<RouteHop>,
    pub total_distance: f64,
}

#[derive(Clone)]
enum EdgeKind {
    Walk,
    Teleport(String),
}

#[derive(Clone)]
struct Edge {
    to: String,
    kind: EdgeKind,
}

/// why: cheap distance-agnostic walk-only graph, built once and cached.
/// Teleport edges deliberately not baked in -- who can use them depends
/// on the player, so they can't share one query-independent graph.
fn walk_graph() -> &'static HashMap<String, Vec<Edge>> {
    static GRAPH: OnceLock<HashMap<String, Vec<Edge>>> = OnceLock::new();
    GRAPH.get_or_init(|| {
        let mut g: HashMap<String, Vec<Edge>> = HashMap::new();
        // why: every zone gets a key up front, even with zero edges --
        // lookup never distinguishes "no key" from "key, empty edge list"
        for z in zonedata::zones() {
            g.entry(z.name.clone()).or_default();
        }

        // why: unioned bidirectionally, resolved through `resolve_zone_name`
        // not a bare exact-name check -- a real reported bug: an exact
        // check silently dropped 29 real adjacency edges (e.g. "Feerrott"
        // vs "The Feerrott") because a zone's adjacent_zones entry
        // routinely uses a shorter/older neighbor name. A genuinely
        // unresolvable name still correctly drops, not invents a node.
        for z in zonedata::zones() {
            for adj in &z.adjacent_zones {
                let Some(resolved) = resolve_zone_name(adj) else {
                    continue;
                };
                g.entry(z.name.clone()).or_default().push(Edge {
                    to: resolved.to_string(),
                    kind: EdgeKind::Walk,
                });
                g.entry(resolved.to_string()).or_default().push(Edge {
                    to: z.name.clone(),
                    kind: EdgeKind::Walk,
                });
            }
        }
        g
    })
}

/// why: cached walk-only base plus teleport edges gated by real player
/// capability. A teleport isn't zone-gated in this game -- castable from
/// anywhere, landing at a fixed spot -- so it's an edge from every zone
/// to the landing zone. Rebuilt per query, cheap enough not to cache.
fn zone_graph_for(player_classes: &[String], player_level: u8) -> HashMap<String, Vec<Edge>> {
    let mut g = walk_graph().clone();
    for (spell, landing) in teleportdata::all_landings() {
        let class_name = landing.class.as_str();
        if !player_classes
            .iter()
            .any(|c| c.eq_ignore_ascii_case(class_name))
        {
            continue;
        }
        if player_level < landing.level {
            continue;
        }
        let Some(dest) = resolve_zone_name(&landing.zone) else {
            continue;
        };
        for z in zonedata::zones() {
            if z.name == dest {
                continue; // why: already there, not a real hop
            }
            g.entry(z.name.clone()).or_default().push(Edge {
                to: dest.to_string(),
                kind: EdgeKind::Teleport(spell.to_string()),
            });
        }
    }
    g
}

/// why: raw non-canonical zone-name strings, same pattern as
/// `zone.rs::ZONE_ALIASES` for a different input shape. Three sources
/// feed this: teleport landing prose (6/105 real entries didn't
/// resolve), adjacent_zones neighbor names, and map exit-marker labels
/// (least clean -- 43 unmatched, some real in-pack zones with classic
/// in-game names, some typos, some word-order swaps).
const ZONE_NAME_ALIASES: &[(&str, &str)] = &[
    ("Cazic Thule", "Cazic Thule (Zone)"),
    ("Temple of Cazic-Thule", "Cazic Thule (Zone)"),
    ("North Karana", "Northern Plains of Karana"),
    ("East Karana", "Eastern Plains of Karana"),
    ("The Southern Plains of Karana", "Southern Karana"),
    ("The Western Plains of Karana", "Western Karana"),
    ("South Ro", "Southern Desert of Ro"),
    ("North Ro", "The Northern Desert of Ro"),
    ("West Karana", "Western Karana"),
    ("The City of Guk", "Upper Guk"),
    ("The Ruins of Old Guk", "Lower Guk"),
    ("The Feerott", "The Feerrott"),
    ("Wakening Lands", "The Wakening Land"),
    ("The Lair of the Splitpaw", "Splitpaw Lair"),
    ("Toxullia Forest", "Toxxulia Forest"),
    ("The Deep", "Timorous Deep"),
    ("The Castle of Mistmoore", "Mistmoore Castle"),
];

/// why: exact match, then alias table, then map-shortname match --
/// lets a bare shortname string ("butcher") resolve without an explicit alias
fn resolve_zone_name(raw: &str) -> Option<&'static str> {
    let zones = zonedata::zones();
    if let Some(z) = zones.iter().find(|z| z.name.eq_ignore_ascii_case(raw)) {
        return Some(z.name.as_str());
    }
    if let Some(&(_, canonical)) = ZONE_NAME_ALIASES
        .iter()
        .find(|&&(alias, _)| alias.eq_ignore_ascii_case(raw))
    {
        if let Some(z) = zones.iter().find(|z| z.name == canonical) {
            return Some(z.name.as_str());
        }
    }
    zones
        .iter()
        .find(|z| {
            z.who_name
                .as_deref()
                .map(zonedata::map_shortnames)
                .unwrap_or_default()
                .iter()
                .any(|s| s.eq_ignore_ascii_case(raw))
        })
        .map(|z| z.name.as_str())
}
/// why: markers whose label plausibly names `target_zone` -- tries
/// `resolve_zone_name` first for exact precision, falls back to loose
/// substring match. Diverges from the frontend's `zoneMatch.ts`, which
/// still only does the loose pass -- a stated gap, not required to fix here.
fn entrance_markers<'a>(
    map: &'a mapsdata::ParsedZoneMap,
    target_zone: &str,
) -> Vec<&'a mapsdata::MapMarker> {
    fn normalize(s: &str) -> String {
        s.chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .map(|c| c.to_ascii_lowercase())
            .collect()
    }
    fn strip_to_prefix(label: &str) -> &str {
        let lower = label.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("to_") {
            &label[label.len() - rest.len()..]
        } else if let Some(rest) = lower.strip_prefix("to ") {
            &label[label.len() - rest.len()..]
        } else {
            label
        }
    }
    let target = normalize(target_zone);
    if target.is_empty() {
        return Vec::new();
    }
    map.markers
        .iter()
        .filter(|m| {
            let readable = strip_to_prefix(&m.label).replace('_', " ");
            if let Some(resolved) = resolve_zone_name(&readable) {
                return resolved.eq_ignore_ascii_case(target_zone);
            }
            let marker = normalize(&readable);
            !marker.is_empty()
                && (marker.starts_with(&target)
                    || target.starts_with(&marker)
                    || marker.contains(&target)
                    || target.contains(&marker))
        })
        .collect()
}

/// why: real walking distance for one edge Dijkstra is relaxing, from
/// `from` to the exit marker toward `to_zone`. Also tries a succor relay
/// and takes whichever is shorter, per direct correction ("2 short stones
/// beats 1 long one"). Falls back to straight-line distance when the
/// map/marker can't resolve or no route exists at all -- an honest
/// approximation, never a silent zero.
///
/// Which real option won -- a direct walk is one hop, a winning succor
/// relay is two (`HopKind::Succor` then the walk from there).
#[derive(Debug, Clone, Copy)]
enum WalkOutcome {
    /// why: real distance straight from `from` to the exit
    Direct(f64),
    /// why: walk distance from the succor point; caller adds SUCCOR_WARP_COST separately
    ViaSuccor(f64),
    /// why: neither real path could be computed (no map/marker, or search failed)
    Fallback(f64),
}

/// why: pulled out as a pure function so it's unit-testable without real
/// map files; `fallback` is a closure so euclidean distance is only paid when needed
fn choose_walk_outcome(
    direct: Option<f64>,
    succor_walk: Option<f64>,
    fallback: impl FnOnce() -> f64,
) -> WalkOutcome {
    match (direct, succor_walk) {
        (Some(a), Some(b)) if b + SUCCOR_WARP_COST < a => WalkOutcome::ViaSuccor(b),
        (Some(a), _) => WalkOutcome::Direct(a),
        (None, Some(b)) => WalkOutcome::ViaSuccor(b),
        (None, None) => WalkOutcome::Fallback(fallback()),
    }
}

/// why: `deadline` checked before every individual find_path call -- one
/// hop_distance call can run several searches (direct + per succor
/// point), measured 4+s on its own even after per-call caps. Returned
/// bool tells `cached_hop_distance` not to permanently cache a
/// time-pressured answer.
fn hop_distance(
    base_dir: &Path,
    from_zone: &str,
    from: (f32, f32, f32),
    to_zone: &str,
    deadline: std::time::Instant,
) -> ((f32, f32, f32), WalkOutcome, bool) {
    let Some(z) = zonedata::zones().iter().find(|z| z.name == from_zone) else {
        return (from, WalkOutcome::Fallback(straight_line_fallback()), false);
    };
    let Some(who_name) = z.who_name.as_deref() else {
        return (from, WalkOutcome::Fallback(straight_line_fallback()), false);
    };
    let shortnames = zonedata::map_shortnames(who_name);
    let Some(shortname) = shortnames.first() else {
        return (from, WalkOutcome::Fallback(straight_line_fallback()), false);
    };
    let Some(map) = cached_map(base_dir, shortname) else {
        return (from, WalkOutcome::Fallback(straight_line_fallback()), false);
    };
    let candidates = entrance_markers(&map, to_zone);
    let Some(marker) = candidates.first() else {
        return (from, WalkOutcome::Fallback(straight_line_fallback()), false);
    };
    let exit = (marker.pos.x, marker.pos.y, marker.pos.z);

    let direct = walk_distance(&map, from, exit);

    // why: succor points carry no Z, so `from`'s own Z stands in
    // (same-floor assumption). Stops trying further points past
    // deadline. Kept as raw walk distance, SUCCOR_WARP_COST added by the caller.
    let mut truncated = false;
    let succor_relay = z
        .succor_evacuate
        .as_deref()
        .map(zonedata::succor_points)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|sp| {
            let past_deadline = std::time::Instant::now() >= deadline; // clock-exempt: real wall-clock search budget, not log time
            if past_deadline {
                truncated = true;
                return None;
            }
            walk_distance(&map, (sp.x, sp.y, from.2), exit)
        })
        .fold(None::<f64>, |best, d| {
            Some(best.map_or(d, |b: f64| b.min(d)))
        });

    let outcome = choose_walk_outcome(direct, succor_relay, || euclid(from, exit));
    (exit, outcome, truncated)
}

/// why: memoized hop_distance, process-lifetime, keyed on (from_zone,
/// from, to_zone) -- sound because `from` is always deterministic here.
/// Quantized to 0.01 so f32 bit-noise can't split one cache entry into two.
fn cached_hop_distance(
    base_dir: &Path,
    from_zone: &str,
    from: (f32, f32, f32),
    to_zone: &str,
    deadline: std::time::Instant,
) -> ((f32, f32, f32), WalkOutcome) {
    type Key = (String, (i64, i64, i64), String);
    type CacheValue = ((f32, f32, f32), WalkOutcome);
    static CACHE: OnceLock<Mutex<HashMap<Key, CacheValue>>> = OnceLock::new();
    fn quantize(v: f32) -> i64 {
        (v as f64 * 100.0).round() as i64
    }
    let key: Key = (
        from_zone.to_string(),
        (quantize(from.0), quantize(from.1), quantize(from.2)),
        to_zone.to_string(),
    );
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(&v) = cache.lock_recover().get(&key) {
        return v;
    }
    let (pos, outcome, truncated) = hop_distance(base_dir, from_zone, from, to_zone, deadline);
    // why: never cache a truncated answer -- it may be worse than a full
    // computation, and this cache is process-lifetime
    if !truncated {
        cache.lock_recover().insert(key, (pos, outcome));
    }
    (pos, outcome)
}

/// why: thin wrapper so callers comparing several options apply their
/// own fallback, rather than hiding which option failed
fn walk_distance(
    map: &mapsdata::ParsedZoneMap,
    from: (f32, f32, f32),
    to: (f32, f32, f32),
) -> Option<f64> {
    let path = pathfind::find_path(map, from, to)?;
    Some(path.windows(2).map(|w| euclid(w[0], w[1])).sum())
}

fn euclid(a: (f32, f32, f32), b: (f32, f32, f32)) -> f64 {
    (((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2) + (a.2 - b.2).powi(2)) as f64).sqrt()
}

/// why: penalized-but-finite so an unresolvable hop loses fairly, not free or crashing
fn straight_line_fallback() -> f64 {
    2000.0
}

/// why: search frontier entry -- `g` is best known real distance,
/// `priority` = g + heuristic, reversed Ord so lowest pops first
struct Frontier {
    priority: f64,
    g: f64,
    zone: String,
}
impl PartialEq for Frontier {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority
    }
}
impl Eq for Frontier {}
impl Ord for Frontier {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .priority
            .partial_cmp(&self.priority)
            .unwrap_or(Ordering::Equal)
    }
}
impl PartialOrd for Frontier {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// why: reverse-BFS hop count to `target`, teleports included. Paired
/// with `HEURISTIC_HOP_COST` this turns the search into weighted A*,
/// biasing the frontier toward the target. Plain Dijkstra (no heuristic)
/// measured 34+s fanning across the entire 105-zone graph before ever
/// settling a real target.
fn hops_to_target(graph: &HashMap<String, Vec<Edge>>, target: &str) -> HashMap<String, usize> {
    let mut incoming: HashMap<&str, Vec<&str>> = HashMap::new();
    for (u, edges) in graph {
        for e in edges {
            incoming.entry(e.to.as_str()).or_default().push(u.as_str());
        }
    }
    let mut dist: HashMap<String, usize> = HashMap::new();
    if !graph.contains_key(target) {
        return dist;
    }
    dist.insert(target.to_string(), 0);
    let mut q = std::collections::VecDeque::from([target]);
    while let Some(z) = q.pop_front() {
        let d = dist[z];
        if let Some(preds) = incoming.get(z) {
            for &u in preds {
                if !dist.contains_key(u) {
                    dist.insert(u.to_string(), d + 1);
                    q.push_back(u);
                }
            }
        }
    }
    dist
}

/// why: typical real hop cost (measured ~1,700-3,500), not the strict
/// admissible minimum (200). Makes this weighted A*, not strict A* --
/// can overestimate, route not provably optimal in every case. The
/// strictly-admissible version still took 30+s (too loose a bound to
/// bias exploration) -- fast and usually-optimal beats provably-exact and slow.
const HEURISTIC_HOP_COST: f64 = 2000.0;

/// why: real walk/teleport route, scored by real distance; None if
/// unconnected. `player_classes`/`player_level` gate which teleports are
/// considered -- empty/0 for walk-only. `known_start`, when Some, is a
/// real confirmed position (a /loc reading or teleport landing), used
/// for the first hop instead of `best_start_position`'s stand-in --
/// strict precision improvement, never required.
///
/// Standard lazy-deletion Dijkstra: a zone's distance finalizes once,
/// the first time popped; `settled` at pop time skips stale heap
/// entries. Each hop starts from the previous hop's fixed exit point,
/// which is what makes `cached_hop_distance` sound.
pub fn find_zone_route(
    base_dir: &Path,
    from_zone: &str,
    to_zone: &str,
    player_classes: &[String],
    player_level: u8,
    known_start: Option<(f32, f32, f32)>,
) -> Option<ZoneRoute> {
    if from_zone == to_zone {
        return Some(ZoneRoute {
            hops: Vec::new(),
            total_distance: 0.0,
        });
    }
    let graph = zone_graph_for(player_classes, player_level);
    if !graph.contains_key(from_zone) || !graph.contains_key(to_zone) {
        return None;
    }
    // why: cheap early-out -- if unreachable in the graph at all, no
    // amount of real pathfinding changes that
    let h_to_target = hops_to_target(&graph, to_zone);
    if !h_to_target.contains_key(from_zone) {
        return None;
    }
    let heuristic = |zone: &str| -> f64 {
        h_to_target
            .get(zone)
            .map(|&h| h as f64 * HEURISTIC_HOP_COST)
            .unwrap_or(f64::INFINITY)
    };

    let mut best_cost: HashMap<String, f64> = HashMap::from([(from_zone.to_string(), 0.0)]);
    let mut best_hops: HashMap<String, Vec<RouteHop>> =
        HashMap::from([(from_zone.to_string(), Vec::new())]);
    let start_pos = known_start.unwrap_or_else(|| best_start_position(base_dir, from_zone));
    let mut best_pos: HashMap<String, (f32, f32, f32)> =
        HashMap::from([(from_zone.to_string(), start_pos)]);
    let mut settled: HashSet<String> = HashSet::new();
    let mut open = BinaryHeap::from([Frontier {
        priority: heuristic(from_zone),
        g: 0.0,
        zone: from_zone.to_string(),
    }]);

    // why: belt-and-suspenders cap -- Dijkstra is already bounded by real
    // node/edge count, this guards against a future graph shape reintroducing
    // an unbounded search
    const MAX_SETTLED: usize = 500;

    // why: teleport edges from every zone give almost everything a low
    // tentative cost, so nearly everything gets settled before a
    // walk-only-reachable target does -- measured 60+s even post-A*-fix
    // for a real dense-teleport query. A wall-clock deadline stops
    // paying for further real walk-edge evaluations (teleport edges stay
    // free) and settles for the best route found so far. 12s comfortably
    // covers the ~5s typical case with margin.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(12); // clock-exempt: real wall-clock search budget, not log time

    while let Some(Frontier { g, zone, .. }) = open.pop() {
        if settled.contains(&zone) {
            continue; // why: stale heap entry, a cheaper path already settled this zone
        }
        settled.insert(zone.clone());
        if zone == to_zone {
            return Some(ZoneRoute {
                hops: best_hops.remove(&zone).unwrap_or_default(),
                total_distance: g,
            });
        }
        if settled.len() > MAX_SETTLED {
            break;
        }

        let Some(edges) = graph.get(&zone) else {
            continue;
        };
        let cur_pos = best_pos[&zone];
        let cur_hops = best_hops[&zone].clone();
        for e in edges {
            if settled.contains(&e.to) {
                continue;
            }
            // why: no path from e.to reaches to_zone -- never worth relaxing
            let Some(&h) = h_to_target.get(e.to.as_str()) else {
                continue;
            };
            // why: checked per-edge not per-zone -- a zone can carry 60+
            // teleport-inflated edges, a per-zone check could overshoot
            let past_deadline = std::time::Instant::now() >= deadline; // clock-exempt: real wall-clock search budget, not log time
            if matches!(e.kind, EdgeKind::Walk) && past_deadline {
                continue;
            }
            // why: a winning succor relay adds two hops, not one -- see HopKind::Succor
            let (new_pos, new_hops) = match &e.kind {
                EdgeKind::Teleport(spell) => {
                    let landing = teleportdata::landing_for(spell)
                        .map(|l| (l.x as f32, l.y as f32, l.z as f32))
                        .unwrap_or(cur_pos);
                    (
                        landing,
                        vec![RouteHop {
                            zone: e.to.clone(),
                            kind: HopKind::Teleport(spell.clone()),
                            distance: TELEPORT_HOP_COST,
                        }],
                    )
                }
                EdgeKind::Walk => {
                    let (arrive_at, outcome) =
                        cached_hop_distance(base_dir, &zone, cur_pos, &e.to, deadline);
                    let hops = match outcome {
                        WalkOutcome::Direct(d) | WalkOutcome::Fallback(d) => {
                            vec![RouteHop {
                                zone: e.to.clone(),
                                kind: HopKind::Walk,
                                distance: d,
                            }]
                        }
                        WalkOutcome::ViaSuccor(walk_dist) => vec![
                            RouteHop {
                                zone: zone.clone(),
                                kind: HopKind::Succor,
                                distance: SUCCOR_WARP_COST,
                            },
                            RouteHop {
                                zone: e.to.clone(),
                                kind: HopKind::Walk,
                                distance: walk_dist,
                            },
                        ],
                    };
                    (arrive_at, hops)
                }
            };
            let added: f64 = new_hops.iter().map(|hop| hop.distance).sum();
            let tentative = g + added;
            if tentative < *best_cost.get(&e.to).unwrap_or(&f64::INFINITY) {
                best_cost.insert(e.to.clone(), tentative);
                let mut hops = cur_hops.clone();
                hops.extend(new_hops);
                best_hops.insert(e.to.clone(), hops);
                best_pos.insert(e.to.clone(), new_pos);
                open.push(Frontier {
                    priority: tentative + h as f64 * HEURISTIC_HOP_COST,
                    g: tentative,
                    zone: e.to.clone(),
                });
            }
        }
    }
    // why: deadline/cap cut the search short -- fall back to the best
    // tentative route, not proven optimal but real and reachable
    best_hops.remove(to_zone).map(|hops| {
        let total = hops.iter().map(|h| h.distance).sum();
        ZoneRoute {
            hops,
            total_distance: total,
        }
    })
}

/// why: "somewhere in this zone" stand-in for the first hop with no live
/// position. Prefers the real succor point (a genuine game-accurate
/// landmark), falls back to wall-line centroid for the 4 zones without
/// one. `pub(crate)` -- `commands::live_start_position` reuses this for
/// Origin (no fixed landing coordinate, only knowable empirically per zone).
pub(crate) fn best_start_position(base_dir: &Path, zone_name: &str) -> (f32, f32, f32) {
    let Some(z) = zonedata::zones().iter().find(|z| z.name == zone_name) else {
        return (0.0, 0.0, 0.0);
    };
    let Some(who_name) = z.who_name.as_deref() else {
        return (0.0, 0.0, 0.0);
    };
    let Some(shortname) = zonedata::map_shortnames(who_name).into_iter().next() else {
        return (0.0, 0.0, 0.0);
    };
    let Some(map) = cached_map(base_dir, &shortname) else {
        return (0.0, 0.0, 0.0);
    };
    let avg_z = if map.lines.is_empty() {
        0.0
    } else {
        map.lines.iter().map(|l| l.a.z + l.b.z).sum::<f32>() / (map.lines.len() * 2) as f32
    };
    if let Some(sp) = z
        .succor_evacuate
        .as_deref()
        .map(zonedata::succor_points)
        .unwrap_or_default()
        .into_iter()
        .next()
    {
        return (sp.x, sp.y, avg_z);
    }
    if map.lines.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    let n = (map.lines.len() * 2) as f32;
    let (sx, sy): (f32, f32) = map.lines.iter().fold((0.0, 0.0), |(sx, sy), l| {
        (sx + l.a.x + l.b.x, sy + l.a.y + l.b.y)
    });
    (sx / n, sy / n, avg_z)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// why: Ak'Anon has exactly one real neighbor -- a direct route must
    /// be one hop; nonexistent base_dir fine, tests hop structure not distance
    #[test]
    fn a_real_direct_adjacency_is_a_single_walk_hop() {
        let route = find_zone_route(
            Path::new("/nonexistent"),
            "Ak'Anon",
            "Steamfont Mountains",
            &[],
            0,
            None,
        )
        .expect("route exists");
        assert_eq!(route.hops.len(), 1, "expected a 1-hop route, got {route:?}");
    }

    /// why: a teleport destination is reachable from an unrelated zone
    /// when the player can actually cast it -- teleports model as "from every zone"
    #[test]
    fn a_real_teleport_spell_is_reachable_when_the_player_can_cast_it() {
        let graph = zone_graph_for(&["Wizard".to_string()], 99);
        let edges = &graph["Ak'Anon"];
        assert!(
            edges
                .iter()
                .any(|e| e.to == "Northern Plains of Karana"
                    && matches!(e.kind, EdgeKind::Teleport(_))),
            "expected a teleport edge from an unrelated zone to Northern Plains of Karana"
        );
    }

    /// why: gating actually gates -- level-1 Wizard and any-level Warrior both excluded
    #[test]
    fn teleport_edges_are_excluded_when_the_player_cannot_cast_them() {
        let too_low_level = zone_graph_for(&["Wizard".to_string()], 1);
        assert!(!too_low_level["Ak'Anon"]
            .iter()
            .any(|e| matches!(&e.kind, EdgeKind::Teleport(s) if s == "North Karana Gate")));

        let wrong_class = zone_graph_for(&["Warrior".to_string()], 99);
        assert!(!wrong_class["Ak'Anon"]
            .iter()
            .any(|e| matches!(&e.kind, EdgeKind::Teleport(s) if s == "North Karana Gate")));

        let no_classes_known = zone_graph_for(&[], 0);
        assert!(no_classes_known["Ak'Anon"]
            .iter()
            .all(|e| matches!(e.kind, EdgeKind::Walk)));
    }

    /// why: must handle all three real shapes -- display name, alias table, bare shortname
    #[test]
    fn resolve_zone_name_handles_all_real_shapes() {
        assert_eq!(
            resolve_zone_name("Northern Plains of Karana"),
            Some("Northern Plains of Karana")
        );
        assert_eq!(
            resolve_zone_name("North Karana"),
            Some("Northern Plains of Karana")
        );
        // why: real Butcherblock Mountains who_name shortname
        assert_eq!(resolve_zone_name("butcher"), Some("Butcherblock Mountains"));
    }

    /// why: "Grimling Forest" genuinely isn't in the scrape -- must resolve None, not guess
    #[test]
    fn a_genuinely_unscraped_zone_name_has_no_resolution() {
        assert_eq!(resolve_zone_name("Grimling Forest"), None);
    }

    /// why: real reported bug -- classic in-game exit labels never
    /// matched modern zone names, silently fell back to the generic
    /// constant. Synthetic map data, testing matching logic not geometry.
    #[test]
    fn classic_in_game_exit_labels_resolve_via_the_alias_table() {
        let map = mapsdata::ParsedZoneMap {
            lines: vec![],
            markers: vec![mapsdata::MapMarker {
                pos: mapsdata::MapPoint3 {
                    x: 1.0,
                    y: 2.0,
                    z: 3.0,
                },
                r: 150,
                g: 0,
                b: 200,
                size: 3,
                label: "to_The_City_of_Guk".to_string(),
            }],
        };
        let found = entrance_markers(&map, "Upper Guk");
        assert_eq!(
            found.len(),
            1,
            "classic name 'The City of Guk' should resolve to Upper Guk"
        );
    }

    #[test]
    fn an_unknown_zone_name_has_no_route() {
        assert!(find_zone_route(
            Path::new("/nonexistent"),
            "Not A Real Zone",
            "Northern Plains of Karana",
            &[],
            0,
            None
        )
        .is_none());
    }

    /// why: real reported bug -- exact-name adjacency matching missed
    /// this pair in both directions, 29 such edges silently dropped pack-wide
    #[test]
    fn cazic_thule_and_feerrott_are_walk_connected_despite_the_real_naming_mismatch() {
        let graph = walk_graph();
        assert!(
            graph["Cazic Thule (Zone)"]
                .iter()
                .any(|e| e.to == "The Feerrott"),
            "Cazic Thule (Zone) should have a walk edge to The Feerrott"
        );
        assert!(
            graph["The Feerrott"]
                .iter()
                .any(|e| e.to == "Cazic Thule (Zone)"),
            "The Feerrott should have a walk edge back to Cazic Thule (Zone)"
        );
    }

    /// why: real reported bug -- dense teleport graph + dead-end target
    /// used to hang the old candidate-generation DFS (app freeze, CPU pegged)
    #[test]
    fn a_dead_end_zone_behind_a_dense_teleport_graph_resolves_quickly() {
        let start = std::time::Instant::now(); // clock-exempt: test, measures its own real elapsed run time
        let route = find_zone_route(
            Path::new("/nonexistent"),
            "The Northern Desert of Ro",
            "Lower Guk",
            &[
                "Wizard".to_string(),
                "Enchanter".to_string(),
                "Magician".to_string(),
            ],
            99,
            None,
        );
        assert!(
            start.elapsed() < std::time::Duration::from_secs(2),
            "took {:?}, should resolve near-instantly",
            start.elapsed()
        );
        let route = route.expect("route exists");
        assert!(
            route
                .hops
                .windows(2)
                .any(|w| w[0].zone == "Upper Guk" && w[1].zone == "Lower Guk")
                || route.hops.last().is_some_and(|h| h.zone == "Lower Guk")
        );
    }

    /// why: regression guard -- a plain 1-hop query used to score several
    /// losing multi-hop alternates too, measured 25s; Dijkstra settles direct
    #[test]
    fn a_direct_adjacency_does_not_pay_for_losing_multihop_alternates() {
        let start = std::time::Instant::now(); // clock-exempt: test, measures its own real elapsed run time
        let route = find_zone_route(
            Path::new("/nonexistent"),
            "The Northern Desert of Ro",
            "Oasis of Marr",
            &[],
            0,
            None,
        );
        assert!(
            start.elapsed() < std::time::Duration::from_secs(1),
            "took {:?}, should resolve near-instantly",
            start.elapsed()
        );
        assert_eq!(route.expect("route exists").hops.len(), 1);
    }

    /// why: real reported bug -- a teleport landing can be a bad spot to
    /// walk from; `choose_walk_outcome`'s two real decision boundaries
    #[test]
    fn succor_relay_wins_only_when_it_genuinely_beats_the_direct_walk() {
        // why: relay (50) + cost (100) = 150, strictly cheaper than 500 -- must win
        assert!(matches!(
            choose_walk_outcome(Some(500.0), Some(50.0), || 0.0),
            WalkOutcome::ViaSuccor(50.0)
        ));
        // why: relay (50) + cost (100) = 150, not cheaper than a short 120 walk -- direct wins
        assert!(matches!(
            choose_walk_outcome(Some(120.0), Some(50.0), || 0.0),
            WalkOutcome::Direct(120.0)
        ));
    }

    #[test]
    fn no_succor_data_falls_back_to_whichever_real_option_exists() {
        assert!(matches!(
            choose_walk_outcome(Some(300.0), None, || 0.0),
            WalkOutcome::Direct(300.0)
        ));
        assert!(matches!(
            choose_walk_outcome(None, Some(80.0), || 0.0),
            WalkOutcome::ViaSuccor(80.0)
        ));
    }

    #[test]
    fn neither_real_option_available_uses_the_fallback() {
        assert!(matches!(
            choose_walk_outcome(None, None, || 2000.0),
            WalkOutcome::Fallback(2000.0)
        ));
    }

    /// why: full-corpus audit per direct ask -- every teleport landing
    /// must resolve or silently vanish from routing; 102/103 do, the one
    /// exception is the already-documented Grimling Forest gap. Asserts
    /// the exact count so a new unresolved landing fails loudly.
    #[test]
    fn every_teleport_landing_resolves_except_the_one_documented_gap() {
        let unresolved: Vec<(&str, &str)> = teleportdata::all_landings()
            .filter(|(_, landing)| resolve_zone_name(&landing.zone).is_none())
            .map(|(spell, landing)| (spell, landing.zone.as_str()))
            .collect();
        assert_eq!(
            unresolved,
            vec![("Grimling Gate", "Grimling Forest")],
            "unresolved teleport landings changed -- see this test's own doc"
        );
    }
}
