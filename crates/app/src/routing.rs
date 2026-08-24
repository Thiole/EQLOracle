//! Zone-to-zone route finding: "how do I get from here to zone X", as a
//! sequence of zone-line crossings and/or teleport shortcuts, weighted by
//! **real in-zone walking distance** (via `pathfind::find_path`), not just
//! hop count -- see `docs/design/maps.md`'s "Pathfinding" section for the
//! full design rationale.
//!
//! **Dijkstra over real distance, not "generate cheap candidates then
//! score them."** An earlier version worked in two blind stages: cheaply
//! enumerate zone-hop sequences by hop count alone, *then* real-score each
//! one. That shape has a real, measured failure: candidate generation
//! doesn't know a 3-hop detour through a huge zone is ever going to lose,
//! so it fully real-pathfinds every hop of it anyway before comparing --
//! confirmed directly, a plain 1-hop `The Northern Desert of Ro` -> `Oasis
//! of Marr` query (real answer: ~2,188 units, one hop) took **25 seconds**
//! against the actual configured install, because the same query also
//! generated and fully real-scored several 2-3 hop alternates through
//! unrelated neighboring zones that could never have won. Dijkstra fixes
//! this structurally, not with a bigger cap: it explores zones in
//! increasing *real accumulated distance* order and finalizes (never
//! re-expands) each one the first time it's popped, so a partial route
//! already costing more than a completed one is never extended further --
//! exactly the "you don't need to see the whole length to discount it"
//! cull the user asked for, not bolted on but the algorithm's own
//! termination property. `to_zone` is provably optimal the instant it's
//! popped; no separate MAX_CANDIDATES/HOP_SLACK bookkeeping is needed
//! (removed), because nothing is ever "one candidate among several" any
//! more -- there is exactly one running best per zone at any time.
//!
//! Real per-hop distance (`hop_distance`, a full `pathfind::find_path`
//! call plus a succor-relay comparison) is still the expensive part, so
//! it's computed lazily -- only when Dijkstra actually settles a zone and
//! relaxes its outgoing edges, never for a whole candidate sequence up
//! front -- and memoized process-lifetime via `cached_hop_distance`
//! (same `OnceLock`-cache shape `cached_map` already uses) keyed on
//! `(from_zone, from_position, to_zone)`, since every walk-hop's `from`
//! position is itself deterministic (either `best_start_position`'s
//! stand-in or a prior hop's own fixed exit/landing point, never a live
//! player coordinate -- see `find_zone_route`'s own doc) -- so the *same*
//! real pathfinding call, if two different explored edges ever need it
//! again, only ever runs once for the life of the process. Teleport edges
//! stay O(1) (a flat `TELEPORT_HOP_COST`, no geometry lookup at all), so
//! only real walk edges ever pay for a `pathfind::find_path` call -- still
//! never a precomputed matrix across all 117 zones, cost stays
//! proportional to what a given query's search frontier actually touches.

use crate::mapsdata;
use crate::pathfind;
use crate::teleportdata;
use crate::zonedata;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

/// A real, measured performance finding, not a hypothetical: a route with
/// several walked hops re-loads and re-parses the same zone's map file
/// every time that zone is visited by any candidate sequence -- a real
/// query (`Ak'Anon` -> `Northern Plains of Karana`, walk-only, 13 real
/// hops) took **12.7s** against the actual configured install before this
/// cache existed. Keyed by shortname only (not `base_dir`) -- the
/// configured install doesn't change mid-session, and this is a plain
/// process-lifetime cache, not something that needs invalidating. `Arc`,
/// not a clone-per-hit, since the largest real zones run 26k+ wall
/// segments (`everfrost.txt`) and copying that on every hop would defeat
/// the point.
fn cached_map(base_dir: &Path, shortname: &str) -> Option<Arc<mapsdata::ParsedZoneMap>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Arc<mapsdata::ParsedZoneMap>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(m) = cache.lock().unwrap().get(shortname) {
        return Some(m.clone());
    }
    let map = Arc::new(mapsdata::load_zone_map(base_dir, None, shortname).ok()?);
    cache
        .lock()
        .unwrap()
        .insert(shortname.to_string(), map.clone());
    Some(map)
}

/// A teleport hop's cost, in the same units as real walking distance --
/// not free (a Wizard/Druid teleport still costs mana and cast time), but
/// small relative to real zone crossings (confirmed real range against
/// the actual configured install: walk hops from ~400 to ~15,000 units)
/// so it reads as the shortcut it actually is. Not derived from anything
/// more precise than "clearly cheaper than walking any real zone,
/// clearly not literally free."
///
/// **Real, stated limitation this cost model does not capture**: it has
/// no notion of whether the querying player can actually *cast* a given
/// teleport spell at all -- every teleport edge in `zone_graph` is
/// unconditionally available, the same way a Wizard's `Gate` and a
/// Druid's `Circle` both appear as options regardless of which class (if
/// either) the player actually plays. A winning route can legitimately
/// require casting several unrelated spells across classes nobody plays
/// at once. This is why `RouteHopDto` tags every teleport hop with its
/// exact spell name rather than folding it into a generic "shortcut" --
/// the frontend surfaces it plainly (see `Maps.svelte`) so the *player*
/// judges viability, rather than the backend silently assuming access to
/// every spell in the game.
const TELEPORT_HOP_COST: f64 = 200.0;

/// The cost of relocating to a zone's own succor/evacuate point -- per
/// the user's own confirmation of the mechanic, this is reachable from
/// *anywhere* in the zone by casting Lesser Evacuate or simply changing
/// the zone's difficulty tier, not gated to a specific class the way
/// Wizard/Druid teleports are (so, unlike `TELEPORT_HOP_COST`, a succor
/// relay never needs a `via_spell` tag -- see `HopKind`'s own doc).
/// Smaller than `TELEPORT_HOP_COST` since it's a same-zone relocation,
/// not a full zone-to-zone jump -- not derived from anything more precise
/// than "a real, discrete action, clearly not free."
const SUCCOR_WARP_COST: f64 = 100.0;

#[derive(Debug, Clone, PartialEq)]
pub enum HopKind {
    Walk,
    /// Cast this spell to make the jump.
    Teleport(String),
    /// Relocate to this zone's own succor/evacuate point (via Lesser
    /// Evacuate or a difficulty-tier change -- see `SUCCOR_WARP_COST`'s
    /// own doc). A real, measured gap this fixes: `hop_distance` always
    /// *computed* whichever was cheaper, direct walk or succor-then-walk,
    /// but used to silently fold a winning succor relay into the walk
    /// hop's own distance number with no indication a real action was
    /// needed to achieve it -- a player who just walked from wherever
    /// they actually landed, without succoring first, would never reach
    /// that number. Now a real, separate, explicit step in the route
    /// (`RouteHop { kind: HopKind::Succor, .. }`, arriving in the *same*
    /// zone the walk that follows it starts from -- succor/difficulty
    /// change repositions within a zone, it never crosses one).
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

/// The cheap, distance-agnostic *walk-only* graph -- nodes are
/// `zonedata::Zone::name` strings. Built once, `OnceLock`-cached, same
/// pattern `zonedata`/`teleportdata` already use for their own embedded
/// data. Teleport edges are deliberately **not** baked in here -- unlike
/// zone adjacency, which is a fixed fact of the world, which teleport
/// edges exist depends on *who's asking* (see `zone_graph_for`'s own doc),
/// so they can't share one cached, query-independent graph.
fn walk_graph() -> &'static HashMap<String, Vec<Edge>> {
    static GRAPH: OnceLock<HashMap<String, Vec<Edge>>> = OnceLock::new();
    GRAPH.get_or_init(|| {
        let mut g: HashMap<String, Vec<Edge>> = HashMap::new();
        // Every real zone gets a key up front, even with zero edges (the 2
        // real zones with no adjacent_zones data at all) -- so a lookup by
        // name never has to distinguish "no key" from "key with an empty
        // edge list", and BFS can always borrow a `&'static str` node
        // reference straight out of this map's own keys (see below) rather
        // than allocating a fresh owned string per visit.
        for z in zonedata::zones() {
            g.entry(z.name.clone()).or_default();
        }

        // Walk edges: adjacent_zones is populated for 115/117 real zones
        // (confirmed against the real pack) -- unioned bidirectionally
        // since the source data isn't guaranteed symmetric in both
        // directions. Resolved through `resolve_zone_name` (same fn
        // `zone_graph_for` already uses for teleport landings), not a bare
        // exact-name check -- a real, measured bug found via the user's
        // own report that a suggested walking route should have gone
        // through `Cazic Thule (Zone)` but never did: an exact-name check
        // silently drops 29 of the pack's own real adjacency edges,
        // because a zone's own `adjacent_zones` entry routinely uses the
        // wiki's shorter/older name for its neighbor (`"Feerrott"` vs the
        // neighbor's own canonical `"The Feerrott"`, `"Cazic Thule"` vs
        // the neighbor's own canonical `"Cazic Thule (Zone)"`) -- the
        // *exact* same naming inconsistency `resolve_zone_name` already
        // exists to solve for `teleportdata::TeleportLanding::zone`
        // strings, just previously never applied here too. Confirmed
        // directly: `Cazic Thule (Zone) <-> The Feerrott` was one of the
        // 29 silently-dropped edges (broken in *both* directions, for two
        // different reasons -- `Cazic Thule (Zone)`'s own listing says
        // `"Feerrott"`, missing "The"; `The Feerrott`'s own listing says
        // `"Cazic Thule"`, missing "(Zone)"), which is exactly why a
        // Cazic-Thule-teleport route into `Lower Guk` was never found: no
        // path existed in the graph at all, not a search/heuristic
        // mistake. A genuinely unresolvable name (wiki scrape junk like
        // `"Wizard"`, or an ambiguous short form with no real match) still
        // correctly drops rather than inventing a dead-end node -- see
        // `resolve_zone_name`'s own doc for what that fallback chain does
        // and doesn't cover.
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

/// One query's actual candidate-generation graph: the cached walk-only
/// base, plus teleport edges filtered to what *this specific player*
/// could actually cast -- `player_classes` (their confirmed class
/// configuration, e.g. from `combat::class_configurations`'s dominant
/// entry) and `player_level` (that configuration's own assumed level,
/// e.g. its `level_range`'s upper bound) gate every teleport edge before
/// it's ever offered as a route option. A Wizard/Druid teleport spell is
/// not zone-gated in this game -- it can be cast from anywhere the caster
/// happens to be standing, landing at a fixed spot in its own destination
/// zone (see `teleportdata`'s own doc) -- so a castable spell isn't a
/// normal A<->B edge, it's an edge *from every zone* to the landing's
/// zone. Rebuilt per query (cloning the cached base graph, ~117 small
/// vecs, then filtering ~103 real landings) rather than cached itself --
/// cheap enough at this scale that caching per-player would just be
/// premature complexity for no measured benefit.
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
                continue; // already there, not a real hop
            }
            g.entry(z.name.clone()).or_default().push(Edge {
                to: dest.to_string(),
                kind: EdgeKind::Teleport(spell.to_string()),
            });
        }
    }
    g
}

/// A raw string that names a zone, but not with its exact canonical
/// `Zone::name` text -- three real sources feed this table, all the same
/// underlying problem (older/shorter/differently-ordered names for a zone
/// than its own wiki-guide title), confirmed by hand against the real
/// pack one entry at a time, the same "small stated exceptions" shape
/// `zone.rs::ZONE_ALIASES` already uses for the analogous log-label-vs-
/// wiki-name problem (a different input shape, so not reused directly,
/// but the same pattern):
/// - `teleportdata::TeleportLanding::zone` (a spell's wiki page prose,
///   e.g. `"North Karana"` for the zone-guide's own `"Northern Plains of
///   Karana"`) -- the original, smaller version of this table only
///   covered this source; only 6 of its 105 real entries failed to
///   resolve at all, one (`"Grimling Forest"`) a genuine upstream gap
///   (isn't in the 117-zone scrape at all).
/// - `Zone::adjacent_zones` entries (one zone's own listing of a
///   neighbor's name, e.g. `"Cazic Thule"` for the neighbor's own
///   canonical `"Cazic Thule (Zone)"`).
/// - Real map-file exit-marker labels (`P ... to_X` lines) -- these
///   turned out to be the least clean source, confirmed directly: 43
///   distinct unmatched labels pack-wide, most genuinely out-of-scope
///   later-expansion zones this 117-zone pack never covers (`Bazaar`,
///   `Nexus`, `Shadowhaven`...), but several real, in-pack zones with a
///   classic in-game name the map author used instead of the wiki's
///   modern title (`"The City of Guk"` for `Upper Guk`, `"The Ruins of
///   Old Guk"` for `Lower Guk` -- this exact pair is what silently broke
///   a real user-reported route through `Lower Guk`, both hops in it
///   fell back to a generic distance constant because neither marker
///   ever matched at all), plain map-author typos (`"Feerott"` missing a
///   letter), or word-order swaps (`"The Castle of Mistmoore"` vs the
///   zone's own `"Mistmoore Castle"`).
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

/// Resolves any of this module's three raw zone-name-ish string shapes
/// (see `ZONE_NAME_ALIASES`'s own doc) to a real `Zone::name`: tries an
/// exact match first, then the alias table, then falls back to matching
/// against each zone's own resolved map shortnames (`zonedata::
/// map_shortnames`, the same resolution `commands::map_zones_for_raw_label`
/// already does for a raw log zone label, reused here for these
/// differently-shaped raw strings) -- this is what lets a bare
/// shortname-looking string (`"butcher"`, `"commons"`) resolve even
/// without an explicit alias entry.
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
/// Real-world (x, y) markers in `zone`'s own map file whose label plausibly
/// names `target_zone`. Tries `resolve_zone_name` first (handles the real,
/// hand-verified cases in `ZONE_NAME_ALIASES` -- classic in-game names,
/// typos, word-order swaps -- with exact precision), falling back to a
/// loose, both-directions substring match on normalized text for anything
/// that doesn't resolve -- a Rust port of `ui/src/lib/maps/
/// zoneMatch.ts::looksLikeEntranceFor`, **not fully in sync with it any
/// more**: the frontend still only does the loose fuzzy match (a separate,
/// smaller-stakes display feature -- which markers get highlighted on the
/// map view -- not the real distances this module computes), so it can
/// now show a marker as a plausible match for a zone name this resolves
/// with real confidence but the frontend's own fuzzy pass wouldn't catch
/// (e.g. `"to_The_City_of_Guk"` for `Upper Guk`). A real, stated
/// divergence, not an oversight -- porting the alias table to
/// `zoneMatch.ts` too is a reasonable follow-up if that display gap ever
/// matters, not required for this module's own real-distance correctness.
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

/// Real walking distance for one walk edge Dijkstra is relaxing, from
/// `from` (a real point already known to be in `from_zone`) to whichever
/// marker in `from_zone`'s own map looks like the exit toward `to_zone`. Also
/// tries relocating to `from_zone`'s own succor/evacuate point(s) first
/// (see `SUCCOR_WARP_COST`'s own doc) and walking from *there* instead,
/// taking whichever real option is actually shorter -- the user's own
/// explicit direction: "I'd rather cross 2 short stones... than 1 really
/// long one", i.e. a succor relocation should win whenever it genuinely
/// shortens the walk, not be ignored in favor of the naive direct route.
/// Falls back to straight-line distance when either the map/marker can't
/// be resolved or `pathfind::find_path` itself finds no route (a real,
/// confirmed possibility, not hypothetical -- see `pathfind.rs`'s own doc
/// for a real zone-line marker that landed in a small, genuinely isolated
/// pocket of a real zone's geometry) -- an honest approximation stated as
/// one, never a silent zero or a dropped candidate.
///
/// Which real option won resolving one walk edge -- everything a caller
/// needs to build the right *number* of `RouteHop`s for it. A direct walk
/// (or the no-real-data fallback) is one hop; a winning succor relay is
/// two, a `HopKind::Succor` hop followed by the walk from there (see
/// `HopKind::Succor`'s own doc for the real bug this fixes -- a winning
/// succor relay used to be silently folded into one walk-hop number with
/// no indication a real in-game action was needed to achieve it).
#[derive(Debug, Clone, Copy)]
enum WalkOutcome {
    /// Real walking distance straight from `from` to the exit.
    Direct(f64),
    /// A succor relay won: the real walk distance *from the succor
    /// point* to the exit. `SUCCOR_WARP_COST` isn't included here -- the
    /// caller adds it as its own separate `HopKind::Succor` hop.
    ViaSuccor(f64),
    /// Neither a direct nor a succor real path could be computed at all
    /// (no map/marker, or every real search failed) -- see `hop_distance`'s
    /// own doc for exactly when.
    Fallback(f64),
}

/// The actual decision -- pulled out of `hop_distance` as its own pure
/// function specifically so it's unit-testable without real map files
/// (`hop_distance` itself needs both a real embedded zone catalog entry
/// and real map files on disk, neither of which a synthetic test can
/// inject -- see this module's own tests for why). `fallback` is a
/// closure, not a plain value, so the (mildly expensive) euclidean
/// distance it computes is never paid unless both real options are
/// unavailable.
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

/// `deadline` is checked before *every* individual `pathfind::find_path`
/// call this makes -- a real, measured bug this fixes: one `hop_distance`
/// call can run several real searches back to back (the direct route,
/// plus one per succor point), so a caller-side deadline check between
/// Dijkstra edges alone can't stop a single pathological zone (several
/// succor points, one or more of them genuinely unreachable and each
/// paying close to `pathfind::find_path`'s own worst case) from blowing
/// past budget entirely -- confirmed directly, one such call took 4+
/// seconds on its own even after that per-call cap was already tightened.
/// The returned `bool` is whether this call actually hit the deadline
/// (skipped at least one real search it would otherwise have run) --
/// `cached_hop_distance` uses it to avoid permanently caching a
/// time-pressured, possibly-worse-than-achievable answer.
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

    // Succor relay: real coordinates have no Z (see `zonedata::
    // SuccorPoint`'s own doc), so `from`'s own Z is reused as a same-floor
    // assumption -- the same kind of approximation `pathfind.rs`'s
    // Z-banding already makes everywhere else in this module. Stops
    // trying further succor points once past deadline, taking whatever
    // real options were already computed rather than starting another
    // potentially-expensive search. Kept as a *raw* walk distance here
    // (no `SUCCOR_WARP_COST` folded in) -- that cost belongs to the
    // caller's own separate `HopKind::Succor` hop, not this number.
    let mut truncated = false;
    let succor_relay = z
        .succor_evacuate
        .as_deref()
        .map(zonedata::succor_points)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|sp| {
            if std::time::Instant::now() >= deadline {
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

/// Memoized `hop_distance` -- process-lifetime cache, same `OnceLock`
/// shape `cached_map` already uses, keyed on `(from_zone, from, to_zone)`.
/// Sound because `from` is always deterministic here (see this module's
/// top doc): either `best_start_position`'s stand-in for the very first
/// hop, or a prior edge's own fixed exit-marker/teleport-landing point --
/// never a live, ever-changing player coordinate. `from` is quantized to
/// the nearest 0.01 unit purely so `f32` bit-pattern noise (there isn't
/// any in practice, since callers always pass through the exact same
/// value, but nothing about `f32` guarantees that) can't split one real
/// cache entry into two.
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
    if let Some(&v) = cache.lock().unwrap().get(&key) {
        return v;
    }
    let (pos, outcome, truncated) = hop_distance(base_dir, from_zone, from, to_zone, deadline);
    // Never cache a deadline-truncated answer -- it may be worse than
    // what a full computation would find, and this cache is process-
    // lifetime (see this fn's own doc), so a stale time-pressured value
    // would otherwise haunt every future query for this exact edge.
    if !truncated {
        cache.lock().unwrap().insert(key, (pos, outcome));
    }
    (pos, outcome)
}

/// Real walking distance between two points on `map`, or `None` if
/// `pathfind::find_path` can't find one -- a thin wrapper so callers
/// comparing several real options (direct walk vs. a succor relay) can
/// each apply their own fallback, rather than one baked-in euclidean
/// substitution hiding which option actually failed.
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

/// Used only when a hop's own zone/map/marker can't be resolved at all --
/// a large, clearly-penalized-but-finite value so a candidate with one
/// unresolvable hop can still lose fairly to a candidate that resolved
/// every hop for real, rather than being silently treated as free or
/// crashing the whole query.
fn straight_line_fallback() -> f64 {
    2000.0
}

/// One entry in the search frontier: `zone`'s best *known* real distance
/// (`g`) from `from_zone` so far, plus `priority` (`g` + a heuristic lower
/// bound on the remaining real distance to the target -- see
/// `hops_to_target`'s own doc) used only for ordering. Reversed `Ord`
/// (like `pathfind::QueueEntry`) since `BinaryHeap` is a max-heap and this
/// search wants the lowest priority popped first.
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

/// Cheap, graph-only reverse-BFS hop count from every zone that can reach
/// `target` (over `graph`'s own edges, teleports included -- a teleport
/// hop still counts as one step closer) to `target` itself. Paired with
/// `HEURISTIC_HOP_COST` (see its own doc for why it's a *typical*, not
/// strictly admissible, per-hop estimate) this is what turns the search in
/// `find_zone_route` into weighted A* instead of plain Dijkstra: `h(zone)
/// = HEURISTIC_HOP_COST * hops_to_target(zone)` biases the frontier
/// (`g + h`) toward zones actually closer to the target, so the search
/// stops wasting real pathfinding calls on zones nowhere near it.
///
/// **Real, measured failure this fixes**: plain Dijkstra (no heuristic at
/// all) against a real walk-only query fanned out across the *entire*
/// 105-zone connected walk graph -- `Great Divide`, `Thurgadin`,
/// `Butcherblock Mountains`, zones on the opposite side of the map --
/// before ever settling `Lower Guk`, because nothing biased exploration
/// toward the actual target; confirmed directly, that query took **34+
/// seconds** and its own real hop-distance trace showed dozens of
/// completely unrelated zones being priced out first.
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

/// The per-hop multiplier `hops_to_target`'s heuristic uses -- deliberately
/// a realistic *typical* real hop cost (matches the bulk of real
/// distances actually measured against the configured install: most
/// real walk hops landed in the ~1,700-3,500 range, not spread evenly
/// across the full documented 400-15,000 extremes), not the strict
/// worst-case-safe *minimum* (`TELEPORT_HOP_COST` = 200).
///
/// **This makes the search weighted A*, not strict A*: the heuristic can
/// overestimate for an unusually cheap real hop, so the route found is
/// not provably optimal in every case.** A real, measured tradeoff, not
/// an oversight: the strictly-admissible version (200/hop) only cut a
/// real walk-only query's settled-zone count from unbounded to 24, and
/// still took 30+ seconds, because 200/hop is such a loose bound against
/// real costs an order of magnitude higher that it barely biased
/// exploration away from plain Dijkstra's behavior. This module already
/// tolerates several other stated, non-exact approximations for the same
/// reason (`TELEPORT_HOP_COST`/`SUCCOR_WARP_COST` are themselves "not
/// derived from anything more precise than a plausible real range") --
/// a fast, honest, usually-optimal-in-practice route beats a
/// provably-exact one that takes 30+ seconds to compute.
const HEURISTIC_HOP_COST: f64 = 2000.0;

/// A real walking-and/or-teleporting route from `from_zone` to `to_zone`,
/// scored by real in-zone distance (see this module's own top doc), or
/// `None` if the two zones aren't connected at all in `zone_graph_for`'s
/// graph. `player_classes`/`player_level` are the querying player's own
/// assumed class configuration -- e.g. `combat::class_configurations(ing,
/// "You")`'s dominant (most zone-visits) entry and that entry's own
/// `level_range` upper bound -- gating which teleport shortcuts are even
/// considered (see `zone_graph_for`'s own doc). Pass an empty slice /
/// `0` to route walk-only, ignoring every teleport shortcut regardless of
/// level.
///
/// `known_start`, when `Some`, is the player's real, confirmed position in
/// `from_zone` right now -- a real `/loc` reading or a confirmed teleport
/// landing (`Ingest::last_loc`/`Ingest::entered_via_teleport`, both
/// map-file-space already), used for the route's own first hop instead of
/// `best_start_position`'s succor-point stand-in. Per the user's own
/// direct point: a zone entered via a recognized teleport cast, or a real
/// `/loc` reading, is 100% known -- exactly the same "confirmed" tier
/// docs/design/maps.md's "You are here" ladder already uses for the map
/// marker, now also the real distance this module reports for the first
/// hop, not just the visual overlay. `None` (a fresh session with no
/// evidence yet, or a query not actually about the player's live position)
/// falls back to the stand-in exactly as before -- this is a strict
/// improvement in precision when available, never a required input.
///
/// Dijkstra over real distance (see this module's top doc for why, and
/// the real freeze this replaced) -- `zone`'s `from_zone`-relative
/// distance is only ever finalized once, the first time it's popped off
/// `open` (standard lazy-deletion Dijkstra: a zone can sit in `open`
/// multiple times at different costs from different relaxations, so a
/// `settled` check at pop time, not a separate "is this entry stale"
/// value comparison, is what skips the redundant ones). Every hop after
/// the first starts from the *previous* hop's own fixed exit/landing
/// point, which is what makes `cached_hop_distance`'s memoization sound.
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
    // Cheap graph-only reachability + heuristic lower bound -- see
    // `hops_to_target`'s own doc. Also a fast, honest early-out: if
    // `to_zone` can't be reached from `from_zone` at all (no walk or
    // teleport chain whatsoever), no amount of real pathfinding will
    // change that, so don't even start the expensive search.
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

    // Belt-and-suspenders cap, same "generous but bounded" shape used
    // throughout this codebase (`pathfind::find_path`'s own
    // `expansion_cap`) -- Dijkstra over this graph is already bounded by
    // real node/edge count (117 zones, a few hundred real walk edges,
    // teleport edges O(1) to relax), so this should never actually bind;
    // it exists so a future graph shape can't silently reintroduce an
    // unbounded search here.
    const MAX_SETTLED: usize = 500;

    // Real, measured problem this guards against: teleport edges are
    // O(1) and unconditionally offered "from every zone" (see
    // `zone_graph_for`'s own doc), so a real multi-teleport-class player
    // config gives almost every one of the 117 real zones a very low
    // tentative cost -- meaning nearly all of them get *settled*, and
    // settling a zone means really pathfinding every one of its real walk
    // edges, before a target that's only walk-reachable (like `Lower
    // Guk`, whose only real neighbor is `Upper Guk`) is ever settled.
    // Confirmed directly against the actual configured install: even
    // after this rewrite and after fixing `pathfind::find_path`'s own
    // worst-case cost, a real `The Northern Desert of Ro` -> `Lower Guk`
    // query with a Wizard/Enchanter/Magician config still hadn't returned
    // after 60+ seconds, because the graph's real walk-edge count (472
    // directed edges total) is large enough that settling "everything
    // cheap-by-teleport first" still means really pathfinding a large
    // share of them. A per-call cost cap bounds *one* call; it can't
    // bound *how many* calls a maximally-connected teleport graph forces.
    // A wall-clock deadline is the honest fix for that: once it passes,
    // stop paying for further real walk-edge evaluations (teleport edges,
    // being free, still relax normally) and settle for the best route
    // found so far to `to_zone`, if any -- not proven optimal in that
    // case, but a real, reachable route rather than an indefinite hang.
    // Real, measured regression from an earlier, too-tight value (2s):
    // before the `hops_to_target`/`HEURISTIC_HOP_COST` weighted-A* fix
    // (see those doc comments), even a *walk-only* query (no teleport
    // edges at all -- a fresh session with no confirmed class yet, since
    // classes never survive an app restart, see `history::reset`'s own
    // doc) fanned out across dozens of irrelevant zones before ever
    // settling `Lower Guk`, and a 2s deadline cut it off before the real
    // 5-hop chain (Northern Ro -> Oasis of Marr -> Southern Desert of Ro
    // -> Innothule Swamp -> Upper Guk -> Lower Guk) ever finished --
    // returning "no route" for a zone the user correctly pointed out is
    // reachable. The A* fix is the real one (cut that same real query
    // from 34s/dozens of zones down to ~5s/6 zones, exactly the zones on
    // the real path), but this deadline stays as the honest backstop for
    // whatever real-world graph shape it hasn't been measured against
    // yet -- 12s comfortably covers the ~5s typical case above with
    // margin, while `hop_distance`'s own per-call deadline check (see its
    // own doc) and `pathfind::find_path`'s tightened expansion cap keep
    // any one pathological edge from eating the whole budget alone.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(12);

    while let Some(Frontier { g, zone, .. }) = open.pop() {
        if settled.contains(&zone) {
            continue; // stale heap entry -- a cheaper path already settled this zone
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
            // No path from `e.to` reaches `to_zone` at all -- never worth
            // relaxing, real cost or not (see `hops_to_target`'s own doc).
            let Some(&h) = h_to_target.get(e.to.as_str()) else {
                continue;
            };
            // Past deadline: teleport edges stay free to relax (O(1), no
            // real cost), but stop paying for further real pathfinding.
            // Checked per-edge, not once per zone -- a single zone can
            // carry 60+ teleport-inflated edges (see `zone_graph_for`'s
            // own doc), so a per-zone check alone could still overshoot
            // the deadline by a whole zone's worth of real calls.
            if matches!(e.kind, EdgeKind::Walk) && std::time::Instant::now() >= deadline {
                continue;
            }
            // Walk edges can add *two* hops, not one -- a winning succor
            // relay (`WalkOutcome::ViaSuccor`) becomes its own explicit
            // `HopKind::Succor` hop, arriving in `zone` (the zone we're
            // already in -- succor/difficulty change repositions within a
            // zone, it never crosses one) followed by the real walk from
            // there. See `HopKind::Succor`'s own doc for why this can't
            // stay one blended number the way it used to.
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
    // Deadline or MAX_SETTLED cut the search short of settling `to_zone`
    // -- fall back to whatever tentative route reached it, if any (not
    // proven optimal, a real reachable route beats no answer at all; see
    // the comment above `deadline`'s own declaration).
    best_hops.remove(to_zone).map(|hops| {
        let total = hops.iter().map(|h| h.distance).sum();
        ZoneRoute {
            hops,
            total_distance: total,
        }
    })
}

/// The best available "somewhere in this zone" stand-in for a route's own
/// first hop, where no live position is part of the query (see
/// `find_zone_route`'s own doc). Prefers the zone's own first real
/// succor/evacuate point -- a genuine, game-accurate "you can always be
/// here" landmark, not a mathematical average that could land inside a
/// wall or an odd corner of the map -- falling back to the wall-line
/// centroid only for the 4 real zones with no usable succor coordinate
/// (see `zonedata::succor_points`'s own doc for exactly which). Succor
/// coordinates carry no Z (see `SuccorPoint`'s own doc), so the wall-line
/// average Z stands in for elevation either way.
///
/// `pub(crate)`, not just internal to this module: `commands::
/// live_start_position` reuses this directly for a real, confirmed case
/// this function's own logic already covers exactly -- Origin (the
/// class-agnostic AA that returns the caster to their starting city) has
/// no fixed, wiki-quotable landing coordinate the way Gate/Translocate/
/// Circle/Ring do (per the user's own point: it's genuinely dynamic, only
/// knowable by observing where a real cast actually lands), but once
/// `Ingest::learned_origin` has empirically confirmed *which zone*,
/// "where in that zone" is exactly this function's own question -- no
/// second implementation needed.
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

    /// Real data: Ak'Anon's own `adjacent_zones` names exactly one real
    /// neighbor (confirmed directly against `packs/zones.json`) -- a
    /// direct route between them must be exactly one walk hop. A
    /// nonexistent `base_dir` is fine here -- `hop_distance` falls back to
    /// `straight_line_fallback` when it can't load a real map, which is
    /// all this test needs (hop *structure*, not real distance).
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

    /// A real teleport spell's destination zone is reachable from an
    /// unrelated, non-adjacent zone in a single teleport hop, *when the
    /// player's assumed class/level actually has it* -- the whole point
    /// of modeling teleports as "from every zone", not a normal adjacency
    /// edge, gated by real capability.
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

    /// The gating actually gates: a level-1 Wizard doesn't have `North
    /// Karana Gate` (real level requirement 18, confirmed against the raw
    /// scrape) yet, and a Warrior never has it at all, regardless of
    /// level -- neither should see the shortcut.
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

    /// `resolve_zone_name` must handle all three real shapes confirmed in
    /// the actual pack: a proper display name, the alias table (the
    /// wiki's own spell-page prose using a shorter name than its
    /// zone-guide title for the same zone), and a bare map-shortname-style
    /// string the wiki left unlinked.
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
        // "butcher" is Butcherblock Mountains' real who_name shortname,
        // confirmed directly against packs/zones.json.
        assert_eq!(resolve_zone_name("butcher"), Some("Butcherblock Mountains"));
    }

    /// The one real, stated gap in `ZONE_NAME_ALIASES`: `"Grimling
    /// Forest"` genuinely isn't in the 117-zone scrape at all -- must
    /// resolve to `None`, not silently guess a near-match.
    #[test]
    fn a_genuinely_unscraped_zone_name_has_no_resolution() {
        assert_eq!(resolve_zone_name("Grimling Forest"), None);
    }

    /// Real, reported bug: a user-suggested route through `Lower Guk`
    /// looked wrong to them, and it was -- both its final two hops
    /// (`Innothule Swamp` -> `Upper Guk` and `Upper Guk` -> `Lower Guk`)
    /// were silently using the generic `straight_line_fallback` constant
    /// instead of a real computed distance, because the real map files'
    /// own exit markers use classic in-game zone names (`"to_The_City_of_
    /// Guk"`, `"to_The_Ruins_of_Old_Guk"`) that never matched `entrance_
    /// markers`' old loose-substring match against the modern `"Upper
    /// Guk"`/`"Lower Guk"` names at all -- confirmed directly against the
    /// real map files. `entrance_markers` now tries `resolve_zone_name`
    /// first specifically to catch cases like this. Synthetic map data
    /// here (not the real install) since this is testing the *matching
    /// logic*, not real geometry.
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

    /// Real, reported bug: the user pointed out a real, valid walking
    /// route through `Cazic Thule (Zone)` that the router never
    /// considered -- traced to `walk_graph` matching a zone's own
    /// `adjacent_zones` entries by *exact* name against the pack's other
    /// zones, when the pack routinely uses a shorter/older name for the
    /// neighbor than that neighbor's own canonical `Zone::name` (the same
    /// inconsistency `resolve_zone_name` already exists to solve for
    /// teleport landings -- `ZONE_NAME_ALIASES` literally lists this
    /// exact pair). Confirmed directly against the real pack: `The
    /// Feerrott`'s own `adjacent_zones` says `"Cazic Thule"` (missing
    /// "(Zone)"), and `Cazic Thule (Zone)`'s own says `"Feerrott"`
    /// (missing "The") -- broken in *both* directions, so the edge was
    /// entirely absent from the graph, not just weighted badly. 29 such
    /// edges were silently dropped pack-wide before this fix (`resolve_
    /// zone_name` reused for adjacency resolution, not just teleport
    /// landings) -- this test is the one the user's own report traced to
    /// directly, not a synthetic stand-in for the general class of bug.
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

    /// Real, reported bug: querying a route toward a zone that's a dead
    /// end behind one narrow real connection (`Lower Guk`'s only
    /// `adjacent_zones` entry is `Upper Guk`), from a start with a dense
    /// teleport-augmented graph (a real multi-teleport-class config makes
    /// `zone_graph_for` put 100+ edges on almost every node), used to spin
    /// the old candidate-generation DFS for minutes/forever hunting for
    /// enough successes through overwhelmingly dead-end branches -- the
    /// user's own report was the whole app freezing with the CPU pegged.
    /// The Dijkstra rewrite (see this module's top doc) removes that
    /// failure mode structurally rather than bounding it, but this test
    /// stays as the regression guard for the *reported symptom*: must
    /// resolve in well under a second and find the real route through
    /// Upper Guk. Nonexistent `base_dir` keeps this portable (no real
    /// install needed) -- see `a_real_direct_adjacency_is_a_single_walk_hop`'s
    /// own comment for why that's fine for a structural assertion like
    /// this one.
    #[test]
    fn a_dead_end_zone_behind_a_dense_teleport_graph_resolves_quickly() {
        let start = std::time::Instant::now();
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

    /// Real, measured bug this fixes, kept as the regression guard: a
    /// plain 1-hop walk-only query used to fully real-score several
    /// 2-3-hop alternates through unrelated zones that could never have
    /// won, because candidate generation was blind to real distance --
    /// 25s against the actual configured install for `The Northern Desert
    /// of Ro` -> `Oasis of Marr` (a real, direct adjacency). Dijkstra
    /// settles the direct 1-hop route and returns as soon as it's popped,
    /// without ever having to enumerate or score the losing alternates at
    /// all. Portable (`/nonexistent` base_dir, see the other tests in
    /// this module for why): asserts the same structural property --
    /// resolves near-instantly and picks the fewest-hops route when nothing
    /// distinguishes real distance (every walk hop falls back to the same
    /// constant), not the real 25s-vs-instant timing itself.
    #[test]
    fn a_direct_adjacency_does_not_pay_for_losing_multihop_alternates() {
        let start = std::time::Instant::now();
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

    /// Real, reported bug: a real teleport spell's landing point (e.g.
    /// `Translocate: Cazic`'s own wizard-spire coordinates) can genuinely
    /// be a bad spot to start walking from -- `hop_distance` always
    /// *computed* the cheaper succor-relay option in that case, but used
    /// to fold it into the walk hop's own number with no indication a
    /// real in-game action (Lesser Evacuate, or a difficulty-tier change)
    /// was needed to actually achieve it. `choose_walk_outcome` is what
    /// decides this now; these are its two real decision boundaries.
    #[test]
    fn succor_relay_wins_only_when_it_genuinely_beats_the_direct_walk() {
        // Succor relay (50) + its cost (100) = 150, strictly cheaper than
        // walking directly (500) -- must win.
        assert!(matches!(
            choose_walk_outcome(Some(500.0), Some(50.0), || 0.0),
            WalkOutcome::ViaSuccor(50.0)
        ));
        // Succor relay (50) + its cost (100) = 150, *not* cheaper than a
        // short direct walk (120) -- must lose, direct wins.
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

    /// Real, full-corpus audit, per the user's own direct ask ("try to go
    /// use all the teleports and make sure they are marked"): every real
    /// teleport spell's landing zone must resolve to a real `Zone::name`
    /// via `resolve_zone_name`, or it silently never becomes a usable
    /// route edge at all (`zone_graph_for` just `continue`s past it, no
    /// error, no log -- a landing that fails to resolve is invisible, not
    /// loud). Confirmed against the real, current pack: 102 of 103
    /// resolve; the one exception (`Grimling Gate` -> `Grimling Forest`)
    /// is the same already-documented, genuine upstream gap
    /// `a_genuinely_unscraped_zone_name_has_no_resolution` covers --
    /// `Grimling Forest` isn't part of this pack's zone scrape at all,
    /// not a resolution bug. Asserts the exact known-good count, not just
    /// "no failures", so a *new* unresolved landing (a future teleport
    /// spell added with a raw zone string this doesn't yet know how to
    /// read) fails loudly here instead of silently vanishing from routing.
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
