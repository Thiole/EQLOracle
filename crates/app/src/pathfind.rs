//! In-zone walking pathfinding over a zone's own wall-segment geometry.
//!
//! See `docs/design/maps.md`'s "Pathfinding" section for the full design
//! rationale. Summary of what this file does and doesn't do:
//!
//! `mapsdata::ParsedZoneMap` is *only* line segments and labeled points --
//! no floor polygons, no rooms, no nav-mesh. There is no existing
//! graph/pathfinding code anywhere in this workspace to build on (checked
//! directly: no crate here pulls in a graph library, and `eqlp_session::
//! graph` is a union-find encounter builder, not a reusable weighted
//! graph -- see that module's own doc). This is new, from scratch.
//!
//! **Z-banded, not true 3D.** These files encode real 3D geometry -- a
//! multi-level dungeon's walls really do occupy very different Z ranges
//! per floor (confirmed: Befallen's own walls span Z -90.6 to +26.1). A
//! flat 2D (x,y) pathfinder would treat a corridor on a different floor as
//! walkable-adjacent. True floor detection (auto-discovering stairs/ramps
//! from line-art alone, with no room/floor data at all) is a
//! research-grade problem this format was never built for. Instead, every
//! query filters wall segments to a Z window around the *starting* point
//! (`Z_BAND`) before building the grid -- pathfinding happens on "the
//! floor you're currently standing on." A route that needs a floor change
//! is a stated, known gap, not silently wrong: `find_path` simply won't
//! find a route across floors it can't see, and returns `None` rather
//! than fabricating one through the wrong level's walls.
//!
//! **Grid A\*, not a visibility graph.** A classic visibility graph (wall
//! endpoints as nodes, edges wherever two endpoints have clear
//! line-of-sight) is the more precise approach for polygon-obstacle
//! shortest paths, but is O(n^2) edge candidates over endpoint count --
//! untenable at real scale (up to 26,383 wall segments in one zone,
//! `everfrost.txt`, confirmed on disk -- 52k+ endpoints). A grid keeps
//! cost bounded by cell count instead, at the price of a resolution
//! tradeoff (a doorway narrower than one cell could be missed) -- tunable
//! via `TARGET_CELLS_PER_AXIS`/`MIN_CELL_SIZE`/`MAX_CELL_SIZE`, not a
//! precision guarantee.

use crate::mapsdata::{MapLine, ParsedZoneMap};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

/// Half-width of the Z window a query's wall filter uses, centered on the
/// *starting* point's own Z -- e.g. a start at Z=0 only sees walls whose
/// segment overlaps [-40, 40]. Not derived from anything more precise than
/// "comfortably smaller than the ~116-unit total Z range a real multi-level
/// dungeon (Befallen) spans, so adjacent floors don't merge into one
/// floor's worth of obstacles" -- a real, stated approximation, not exact
/// per-zone floor geometry (which nothing in this data format states).
const Z_BAND: f32 = 40.0;

/// Roughly how many grid cells the *longer* axis of a zone's bounding box
/// is divided into -- the actual cell size this produces varies per zone
/// (a huge outdoor zone gets bigger cells than a small building), clamped
/// by `MIN_CELL_SIZE`/`MAX_CELL_SIZE` so neither a tiny zone (cells too
/// small to be worth the node count) nor a huge one (cells so big real
/// geometry gets skipped over) is pathological.
const TARGET_CELLS_PER_AXIS: f32 = 250.0;
const MIN_CELL_SIZE: f32 = 8.0;
const MAX_CELL_SIZE: f32 = 200.0;

/// Real, checked finding, not a guess: **not every `L` line is a wall**.
/// Treating every line in an outdoor zone as a hard obstacle produces
/// genuinely wrong results -- confirmed directly against the real
/// `northkarana.txt`: two of its three actual zone-line exit markers came
/// back unreachable from the third via a full 2D flood-fill (not a
/// pathfinding bug -- 81% of the zone's open cells *were* one connected
/// component, but those two markers sat in the disconnected 19%). Sampling
/// every real color used in that file found five: pure black (5,514
/// lines), gray `128,128,128` (622 -- both already established elsewhere
/// in this codebase as real wall colors, see `MapViewer.svelte`'s own
/// comment), two brown/dirt tones (`160,120,60` and `100,50,0`, 2,425
/// lines combined -- terrain/ground-contour art, not obstacles), a blue
/// (`0,0,255`, 196 lines -- water, swimmable in this game, not a hard
/// wall), and a distinct magenta (`150,0,200`, exactly 3 lines -- one per
/// real zone-line exit, clearly the zone-boundary marker itself, which
/// must never block the very route leading to it). Excluding the
/// non-grayscale colors and re-running the same flood-fill fixed one of
/// the two disconnected markers outright; the third sits inside a real,
/// narrow, purpose-built gate structure (confirmed by inspecting its
/// nearby geometry directly) that a coarse grid can still miss -- a
/// separate, already-documented resolution tradeoff, not something this
/// color filter was expected to fix too.
fn is_wall_color(r: u8, g: u8, b: u8) -> bool {
    r == g && g == b
}

type Cell = (i32, i32);

struct Grid {
    origin_x: f32,
    origin_y: f32,
    cell: f32,
    /// Wall segments (already Z-filtered), bucketed by every grid cell
    /// their own XY bounding box (expanded one cell for safety at cell
    /// boundaries) touches. Only used for adjacency-edge blocking checks
    /// between cells that are already known to be near each other -- never
    /// walked as a full line-rasterization, since A* only ever asks "is
    /// the edge between these two *adjacent* cells blocked", not
    /// long-range visibility.
    buckets: HashMap<Cell, Vec<usize>>,
    segments: Vec<(f32, f32, f32, f32)>,
}

impl Grid {
    fn build(lines: &[MapLine], ref_z: f32) -> Self {
        let z_lo = ref_z - Z_BAND;
        let z_hi = ref_z + Z_BAND;
        let segments: Vec<(f32, f32, f32, f32)> = lines
            .iter()
            .filter(|l| {
                let (lo, hi) = (l.a.z.min(l.b.z), l.a.z.max(l.b.z));
                lo <= z_hi && hi >= z_lo
            })
            .filter(|l| is_wall_color(l.r, l.g, l.b_))
            .map(|l| (l.a.x, l.a.y, l.b.x, l.b.y))
            .collect();

        let (mut min_x, mut min_y, mut max_x, mut max_y) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for &(x1, y1, x2, y2) in &segments {
            min_x = min_x.min(x1).min(x2);
            max_x = max_x.max(x1).max(x2);
            min_y = min_y.min(y1).min(y2);
            max_y = max_y.max(y1).max(y2);
        }
        if !min_x.is_finite() {
            (min_x, min_y, max_x, max_y) = (ref_z, ref_z, ref_z, ref_z);
        }

        let span = (max_x - min_x).max(max_y - min_y).max(1.0);
        let cell = (span / TARGET_CELLS_PER_AXIS).clamp(MIN_CELL_SIZE, MAX_CELL_SIZE);

        let mut grid = Grid {
            origin_x: min_x,
            origin_y: min_y,
            cell,
            buckets: HashMap::new(),
            segments,
        };
        for (idx, &(x1, y1, x2, y2)) in grid.segments.iter().enumerate() {
            let (c1i, c1j) = grid.cell_of(x1, y1);
            let (c2i, c2j) = grid.cell_of(x2, y2);
            let (lo_i, hi_i) = (c1i.min(c2i) - 1, c1i.max(c2i) + 1);
            let (lo_j, hi_j) = (c1j.min(c2j) - 1, c1j.max(c2j) + 1);
            for i in lo_i..=hi_i {
                for j in lo_j..=hi_j {
                    grid.buckets.entry((i, j)).or_default().push(idx);
                }
            }
        }
        grid
    }

    fn cell_of(&self, x: f32, y: f32) -> Cell {
        (
            ((x - self.origin_x) / self.cell).floor() as i32,
            ((y - self.origin_y) / self.cell).floor() as i32,
        )
    }

    fn cell_center(&self, c: Cell) -> (f32, f32) {
        (
            self.origin_x + (c.0 as f32 + 0.5) * self.cell,
            self.origin_y + (c.1 as f32 + 0.5) * self.cell,
        )
    }

    /// A cell counts as "open" if at least one of its own 8 neighbor edges
    /// isn't blocked -- a cheap proxy for "this cell can actually
    /// participate in a path" (vs. fully enclosed on every side).
    fn is_open(&self, c: Cell) -> bool {
        let (cx, cy) = self.cell_center(c);
        NEIGHBOR_OFFSETS.iter().any(|&(di, dj)| {
            let (nx, ny) = self.cell_center((c.0 + di, c.1 + dj));
            !self.blocked(cx, cy, nx, ny)
        })
    }

    /// Real zone-line exit markers (and other points of interest) are
    /// often placed right against the very wall/boundary geometry they
    /// mark -- confirmed directly against real data: querying a route to
    /// a real "to_<Zone>" marker's own exact coordinate in North Karana
    /// failed outright (every neighboring cell blocked) even though every
    /// point a few percent short of it, along the same line, succeeded.
    /// Rather than reporting a point genuinely walkable in the real game
    /// as unreachable, spiral outward in ring order from `c` and use the
    /// first open cell found -- the same "snap to the nearest usable
    /// point" mitigation navmesh libraries apply for a query point that
    /// lands exactly on or inside obstacle geometry.
    fn nearest_open_cell(&self, c: Cell, max_radius: i32) -> Option<Cell> {
        if self.is_open(c) {
            return Some(c);
        }
        for radius in 1..=max_radius {
            for di in -radius..=radius {
                for dj in -radius..=radius {
                    if di.abs() != radius && dj.abs() != radius {
                        continue; // only the current ring's perimeter
                    }
                    let cand = (c.0 + di, c.1 + dj);
                    if self.is_open(cand) {
                        return Some(cand);
                    }
                }
            }
        }
        None
    }

    /// Whether the straight segment `(x1,y1)-(x2,y2)` crosses any wall
    /// segment bucketed near either endpoint -- real segment-segment
    /// intersection, not a "is this cell inside a wall" test, since these
    /// files are open line obstacles (wall outlines), not filled polygons.
    fn blocked(&self, x1: f32, y1: f32, x2: f32, y2: f32) -> bool {
        let c1 = self.cell_of(x1, y1);
        let c2 = self.cell_of(x2, y2);
        let mut checked = std::collections::HashSet::new();
        for c in [c1, c2] {
            let Some(bucket) = self.buckets.get(&c) else {
                continue;
            };
            for &idx in bucket {
                if !checked.insert(idx) {
                    continue;
                }
                let (wx1, wy1, wx2, wy2) = self.segments[idx];
                if segments_intersect(x1, y1, x2, y2, wx1, wy1, wx2, wy2) {
                    return true;
                }
            }
        }
        false
    }
}

/// Standard orientation-based segment-segment intersection test (proper
/// crossing or an endpoint landing exactly on the other segment).
fn segments_intersect(
    ax1: f32,
    ay1: f32,
    ax2: f32,
    ay2: f32,
    bx1: f32,
    by1: f32,
    bx2: f32,
    by2: f32,
) -> bool {
    fn orient(ax: f32, ay: f32, bx: f32, by: f32, cx: f32, cy: f32) -> f32 {
        (bx - ax) * (cy - ay) - (by - ay) * (cx - ax)
    }
    // Only ever called once the caller has confirmed collinearity (d == 0
    // at the call site) -- a plain bounding-box containment check is
    // sufficient at that point.
    fn on_segment(ax: f32, ay: f32, bx: f32, by: f32, px: f32, py: f32) -> bool {
        px >= ax.min(bx) && px <= ax.max(bx) && py >= ay.min(by) && py <= ay.max(by)
    }
    let d1 = orient(bx1, by1, bx2, by2, ax1, ay1);
    let d2 = orient(bx1, by1, bx2, by2, ax2, ay2);
    let d3 = orient(ax1, ay1, ax2, ay2, bx1, by1);
    let d4 = orient(ax1, ay1, ax2, ay2, bx2, by2);
    if ((d1 > 0.0) != (d2 > 0.0)) && ((d3 > 0.0) != (d4 > 0.0)) {
        return true;
    }
    (d1 == 0.0 && on_segment(bx1, by1, bx2, by2, ax1, ay1))
        || (d2 == 0.0 && on_segment(bx1, by1, bx2, by2, ax2, ay2))
        || (d3 == 0.0 && on_segment(ax1, ay1, ax2, ay2, bx1, by1))
        || (d4 == 0.0 && on_segment(ax1, ay1, ax2, ay2, bx2, by2))
}

#[derive(PartialEq)]
struct QueueEntry {
    cost: f32,
    cell: Cell,
}
impl Eq for QueueEntry {}
impl Ord for QueueEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reversed: BinaryHeap is a max-heap, A* wants the lowest cost first.
        other
            .cost
            .partial_cmp(&self.cost)
            .unwrap_or(Ordering::Equal)
    }
}
impl PartialOrd for QueueEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

const NEIGHBOR_OFFSETS: [(i32, i32); 8] = [
    (1, 0),
    (-1, 0),
    (0, 1),
    (0, -1),
    (1, 1),
    (1, -1),
    (-1, 1),
    (-1, -1),
];

/// A real walking route from `from` to `to` within `map`, or `None` if no
/// route exists (unreachable on the same floor, or `from`/`to` themselves
/// sit inside blocked geometry). Z-banded around `from`'s own Z -- see this
/// module's own top doc. Output is real-world (x, y, z) waypoints,
/// simplified via line-of-sight string-pulling so the drawn path is a
/// small number of real turns, not a jagged staircase of every grid cell
/// visited.
pub fn find_path(
    map: &ParsedZoneMap,
    from: (f32, f32, f32),
    to: (f32, f32, f32),
) -> Option<Vec<(f32, f32, f32)>> {
    let grid = Grid::build(&map.lines, from.2);
    // why: snap both endpoints to the nearest open cell before searching --
    // see `nearest_open_cell`'s own doc for the real case this fixes (a
    // query point sitting right against wall/boundary geometry, like a
    // real zone-line marker). A generous radius (20 cells, hundreds of
    // real units) since this should only ever need to step a short way
    // off genuinely enclosing geometry, not silently substitute a wildly
    // different point for a truly-unreachable one.
    const SNAP_RADIUS: i32 = 20;
    let start = grid.nearest_open_cell(grid.cell_of(from.0, from.1), SNAP_RADIUS)?;
    let goal = grid.nearest_open_cell(grid.cell_of(to.0, to.1), SNAP_RADIUS)?;

    if start == goal {
        return Some(vec![from, to]);
    }

    let heuristic = |c: Cell| -> f32 {
        let (cx, cy) = grid.cell_center(c);
        let (gx, gy) = grid.cell_center(goal);
        ((cx - gx).powi(2) + (cy - gy).powi(2)).sqrt()
    };

    let mut open = BinaryHeap::new();
    open.push(QueueEntry {
        cost: heuristic(start),
        cell: start,
    });
    let mut g_score: HashMap<Cell, f32> = HashMap::from([(start, 0.0)]);
    let mut came_from: HashMap<Cell, Cell> = HashMap::new();
    // Safety cap: a genuinely unreachable goal on a huge zone should fail
    // *fast*, not scan forever -- and the old flat 2,000,000 didn't:
    // measured directly against the actual configured install's biggest
    // real zone (`everfrost.txt`, 26,383 wall segments), a genuinely
    // unreachable query hit the full cap and took **14.3 seconds** doing
    // it, on one single call -- routing queries that touch several zones
    // (`routing::find_zone_route`) could stack several of these, which is
    // what the user's own reported "the whole app freezes" traced back to.
    // The real number that matters, also measured directly in the same
    // zone: the largest legitimate real route this format can produce (a
    // reachable corner-to-corner traverse of that same 26,383-segment
    // zone) only ever needed 49,533 expansions. 75,000 keeps a ~1.5x
    // margin over that real measured need for the single biggest real
    // zone in the game (every other zone is smaller, so has more relative
    // headroom) while bounding a genuinely unreachable query's worst case
    // to well under a second -- `routing::find_zone_route`'s own weighted-
    // A* search still touches several real edges per query even when it's
    // working well (see that module's own doc), so keeping each
    // individual real call cheap matters more here than maximizing
    // headroom on the one biggest zone's rare worst case.
    let expansion_cap = 75_000;
    let mut expansions = 0;

    while let Some(QueueEntry { cell: current, .. }) = open.pop() {
        if current == goal {
            return Some(reconstruct_path(&grid, &came_from, current, from, to));
        }
        expansions += 1;
        if expansions > expansion_cap {
            return None;
        }
        let current_g = g_score[&current];
        let (cx, cy) = grid.cell_center(current);
        for (di, dj) in NEIGHBOR_OFFSETS {
            let next = (current.0 + di, current.1 + dj);
            let (nx, ny) = grid.cell_center(next);
            if grid.blocked(cx, cy, nx, ny) {
                continue;
            }
            let step_cost = if di != 0 && dj != 0 {
                std::f32::consts::SQRT_2
            } else {
                1.0
            } * grid.cell;
            let tentative = current_g + step_cost;
            if tentative < *g_score.get(&next).unwrap_or(&f32::INFINITY) {
                g_score.insert(next, tentative);
                came_from.insert(next, current);
                open.push(QueueEntry {
                    cost: tentative + heuristic(next),
                    cell: next,
                });
            }
        }
    }
    None
}

fn reconstruct_path(
    grid: &Grid,
    came_from: &HashMap<Cell, Cell>,
    goal: Cell,
    from: (f32, f32, f32),
    to: (f32, f32, f32),
) -> Vec<(f32, f32, f32)> {
    let mut cells = vec![goal];
    let mut cur = goal;
    while let Some(&prev) = came_from.get(&cur) {
        cells.push(prev);
        cur = prev;
    }
    cells.reverse();

    let mut points: Vec<(f32, f32, f32)> = vec![from];
    points.extend(cells.iter().map(|&c| {
        let (x, y) = grid.cell_center(c);
        (x, y, from.2)
    }));
    points.push((to.0, to.1, to.2));

    simplify(grid, &points)
}

/// Line-of-sight string-pulling: keep an anchor point, extend forward as
/// far as still-unblocked straight-line travel allows, drop everything in
/// between. Turns a jagged per-cell path into a small number of real
/// turns -- what a player would actually walk, not a staircase.
fn simplify(grid: &Grid, points: &[(f32, f32, f32)]) -> Vec<(f32, f32, f32)> {
    if points.len() <= 2 {
        return points.to_vec();
    }
    let mut out = vec![points[0]];
    let mut probe = 2;
    while probe < points.len() {
        let (ax, ay, _) = out[out.len() - 1];
        let (px, py, _) = points[probe];
        // Blocked from the current anchor -> commit the last point that
        // *was* visible (guaranteed unblocked: it's an original
        // grid-adjacent step from the A* search itself) as the new anchor.
        if grid.blocked(ax, ay, px, py) {
            out.push(points[probe - 1]);
        }
        probe += 1;
    }
    out.push(points[points.len() - 1]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mapsdata::MapPoint3;

    fn line(ax: f32, ay: f32, bx: f32, by: f32) -> MapLine {
        MapLine {
            a: MapPoint3 {
                x: ax,
                y: ay,
                z: 0.0,
            },
            b: MapPoint3 {
                x: bx,
                y: by,
                z: 0.0,
            },
            r: 0,
            g: 0,
            b_: 0,
        }
    }

    /// An open square room, no obstacles inside -- a straight-line-shaped
    /// route from one corner to the other should exist and both endpoints
    /// should round-trip.
    #[test]
    fn a_clear_room_finds_a_direct_route() {
        let lines = vec![
            line(-100.0, -100.0, 100.0, -100.0),
            line(100.0, -100.0, 100.0, 100.0),
            line(100.0, 100.0, -100.0, 100.0),
            line(-100.0, 100.0, -100.0, -100.0),
        ];
        let map = ParsedZoneMap {
            lines,
            markers: vec![],
        };
        let path = find_path(&map, (-80.0, -80.0, 0.0), (80.0, 80.0, 0.0)).expect("route exists");
        assert_eq!(path.first().copied(), Some((-80.0, -80.0, 0.0)));
        assert_eq!(path.last().copied(), Some((80.0, 80.0, 0.0)));
    }

    /// A single solid wall straight down the middle of the room, no gap --
    /// the two sides are genuinely unreachable from each other, and
    /// `find_path` must say so rather than walking through it.
    #[test]
    fn a_wall_with_no_gap_blocks_the_route() {
        // A closed room (so there's no way to walk *around* the dividing
        // wall's ends, only through it) with a full-height dividing wall
        // and no gap at all.
        let lines = vec![
            line(-100.0, -100.0, 100.0, -100.0),
            line(100.0, -100.0, 100.0, 100.0),
            line(100.0, 100.0, -100.0, 100.0),
            line(-100.0, 100.0, -100.0, -100.0),
            line(0.0, -100.0, 0.0, 100.0),
        ];
        let map = ParsedZoneMap {
            lines,
            markers: vec![],
        };
        assert!(find_path(&map, (-50.0, 0.0, 0.0), (50.0, 0.0, 0.0)).is_none());
    }

    /// Same dividing wall, but with a real gap in it -- the route must
    /// exist and must actually go through the gap, not in a straight line
    /// through the solid parts of the wall on either side of it.
    #[test]
    fn a_wall_with_a_gap_routes_through_the_gap() {
        // Same enclosed-room shape as the no-gap test, but the dividing
        // wall has a real 40-unit gap in the middle -- going around the
        // *outside* of the room is impossible (it's fully closed), so a
        // found route necessarily proves the gap itself got used, not just
        // that a route was returned.
        let lines = vec![
            line(-100.0, -100.0, 100.0, -100.0),
            line(100.0, -100.0, 100.0, 100.0),
            line(100.0, 100.0, -100.0, 100.0),
            line(-100.0, 100.0, -100.0, -100.0),
            line(0.0, -100.0, 0.0, -20.0),
            line(0.0, 20.0, 0.0, 100.0),
        ];
        let map = ParsedZoneMap {
            lines,
            markers: vec![],
        };
        let path =
            find_path(&map, (-50.0, 0.0, 0.0), (50.0, 0.0, 0.0)).expect("route exists via the gap");
        // Every consecutive leg of the simplified path must itself be
        // unobstructed -- the real assertion that this isn't just "a path
        // was returned" but "a path that actually respects the wall".
        let grid = Grid::build(&map.lines, 0.0);
        for w in path.windows(2) {
            assert!(
                !grid.blocked(w[0].0, w[0].1, w[1].0, w[1].1),
                "leg {:?}->{:?} crosses the wall",
                w[0],
                w[1]
            );
        }
    }

    /// Two rooms stacked on different floors (same X/Y footprint, Z
    /// separated by more than Z_BAND) -- querying from the lower floor
    /// must never route through the upper floor's geometry as if it were
    /// on the same level. Documents the stated Z-banding limitation with
    /// a real, checked test rather than just a comment.
    #[test]
    fn a_different_floor_at_the_same_xy_is_not_treated_as_the_same_level() {
        let lower = vec![line(-100.0, -100.0, 100.0, -100.0)];
        let upper = vec![MapLine {
            a: MapPoint3 {
                x: 0.0,
                y: -200.0,
                z: 500.0,
            },
            b: MapPoint3 {
                x: 0.0,
                y: 200.0,
                z: 500.0,
            },
            r: 0,
            g: 0,
            b_: 0,
        }];
        let mut lines = lower;
        lines.extend(upper);
        let map = ParsedZoneMap {
            lines,
            markers: vec![],
        };
        // Both points are on the lower floor (Z=0); the upper-floor wall
        // (Z=500) is far outside Z_BAND and must be filtered out, so this
        // route must succeed exactly as if the upper wall didn't exist.
        assert!(find_path(&map, (-50.0, 0.0, 0.0), (50.0, 0.0, 0.0)).is_some());
    }
}
