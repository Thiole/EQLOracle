//! why: in-zone walking pathfinding over a zone's own wall-segment geometry
//!
//! See `docs/design/maps.md`'s "Pathfinding" section for the full
//! rationale. No graph library or reusable graph code existed in this
//! workspace -- built from scratch.
//!
//! **Z-banded, not true 3D.** A multi-level dungeon's walls occupy very
//! different Z ranges per floor (Befallen: -90.6 to +26.1). A flat 2D
//! pathfinder would treat a different floor's corridor as adjacent; true
//! floor detection from line-art alone is research-grade, out of scope.
//! Each query filters walls to a Z window around the starting point
//! instead -- a floor-change route is a stated gap, returns None rather
//! than fabricating a route through the wrong level.
//!
//! **Grid A\*, not a visibility graph.** A visibility graph is O(n^2)
//! over endpoint count -- untenable at real scale (26,383 segments in
//! one real zone, 52k+ endpoints). A grid bounds cost by cell count
//! instead, trading resolution (a narrow doorway could be missed).

use crate::mapsdata::{MapLine, ParsedZoneMap};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

/// why: half-width of the Z window around the start point; smaller than
/// Befallen's ~116-unit total floor range so adjacent floors don't merge
const Z_BAND: f32 = 40.0;

/// why: roughly how many cells the longer bounding-box axis divides
/// into; clamped so neither a tiny nor huge zone is pathological
const TARGET_CELLS_PER_AXIS: f32 = 250.0;
const MIN_CELL_SIZE: f32 = 8.0;
const MAX_CELL_SIZE: f32 = 200.0;

/// why: not every `L` line is a wall -- confirmed against real
/// `northkarana.txt` where two of three zone-line exits were
/// unreachable via flood-fill. Grayscale (black/gray) are real walls;
/// brown/dirt tones are terrain art, blue is swimmable water, magenta
/// marks the zone-line exit itself -- filtering to grayscale-only fixed
/// one of the two disconnected markers; the third sits in a genuinely
/// narrow gate structure, a separate resolution tradeoff.
fn is_wall_color(r: u8, g: u8, b: u8) -> bool {
    r == g && g == b
}

type Cell = (i32, i32);

struct Grid {
    origin_x: f32,
    origin_y: f32,
    cell: f32,
    /// why: Z-filtered walls bucketed by touched cell (expanded 1 for
    /// safety); only used for adjacency-edge checks, not long-range visibility
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

    /// why: open if at least one of 8 neighbor edges isn't blocked -- cheap
    /// proxy for "can participate in a path"
    fn is_open(&self, c: Cell) -> bool {
        let (cx, cy) = self.cell_center(c);
        NEIGHBOR_OFFSETS.iter().any(|&(di, dj)| {
            let (nx, ny) = self.cell_center((c.0 + di, c.1 + dj));
            !self.blocked(cx, cy, nx, ny)
        })
    }

    /// why: zone-line exit markers often sit right against wall geometry
    /// -- confirmed a real North Karana marker failed outright at its
    /// exact coordinate. Spirals outward and snaps to the first open
    /// cell, the standard navmesh mitigation for this.
    fn nearest_open_cell(&self, c: Cell, max_radius: i32) -> Option<Cell> {
        if self.is_open(c) {
            return Some(c);
        }
        for radius in 1..=max_radius {
            for di in -radius..=radius {
                for dj in -radius..=radius {
                    if di.abs() != radius && dj.abs() != radius {
                        continue; // why: only the current ring's perimeter
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

    /// why: real segment-segment intersection, not "inside a wall" --
    /// these files are open line obstacles, not filled polygons
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

/// why: standard orientation-based intersection test, proper crossing or endpoint-on-segment
#[allow(clippy::too_many_arguments)] // two raw (x,y) endpoint pairs, not a natural struct
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
    // why: only called after the caller confirms collinearity -- bounding-box check suffices
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
        // why: reversed -- BinaryHeap is a max-heap, A* wants lowest cost first
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

/// why: real walking route or None if unreachable; Z-banded around
/// `from`; simplified via string-pulling to a small number of real turns
pub fn find_path(
    map: &ParsedZoneMap,
    from: (f32, f32, f32),
    to: (f32, f32, f32),
) -> Option<Vec<(f32, f32, f32)>> {
    let grid = Grid::build(&map.lines, from.2);
    // why: snap endpoints to nearest open cell -- see `nearest_open_cell`;
    // radius generous enough for a short step off enclosing geometry,
    // not so large it substitutes a wildly different point
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
    // why: safety cap -- old flat 2,000,000 took 14.3s on an unreachable
    // query in the biggest real zone (everfrost.txt), traced to the
    // user's reported app freeze. 75,000 keeps ~1.5x margin over the
    // largest legitimate real route's measured 49,533 expansions.
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

/// why: string-pulling -- extend an anchor as far as unblocked, drop the rest
fn simplify(grid: &Grid, points: &[(f32, f32, f32)]) -> Vec<(f32, f32, f32)> {
    if points.len() <= 2 {
        return points.to_vec();
    }
    let mut out = vec![points[0]];
    let mut probe = 2;
    while probe < points.len() {
        let (ax, ay, _) = out[out.len() - 1];
        let (px, py, _) = points[probe];
        // why: blocked -- commit the last visible point (guaranteed
        // unblocked, an original grid-adjacent A* step) as the new anchor
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

    /// why: open room, no obstacles -- a direct route exists, endpoints round-trip
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

    /// why: a solid dividing wall, no gap -- the two sides are genuinely unreachable
    #[test]
    fn a_wall_with_no_gap_blocks_the_route() {
        // why: closed room, no way around the dividing wall's ends, only through it
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

    /// why: same wall with a real gap -- route must exist and use the gap
    #[test]
    fn a_wall_with_a_gap_routes_through_the_gap() {
        // why: fully enclosed room, going around the outside is impossible,
        // so a found route proves the gap itself got used
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
        // why: every leg must itself be unobstructed -- not just "a path was returned"
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

    /// why: two rooms on different floors, same XY -- must never route through the upper one
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
        // why: upper wall is far outside Z_BAND, must be filtered out entirely
        assert!(find_path(&map, (-50.0, 0.0, 0.0), (50.0, 0.0, 0.0)).is_some());
    }
}
