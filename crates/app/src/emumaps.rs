//! EQEmu navigation data: navmesh pathfinding and best-Z ground snapping.
//!
//! Sources (https://github.com/EQEmu/maps): `nav/<zone>.nav` -- a
//! Recast/Detour tiled navmesh in an `EQNAVMESH` zlib wrapper, the
//! walkable-surface graph the emulator's own NPCs path on;
//! `base/<zone>.map` -- the zone's collision triangle mesh (`best_z`).
//! The game's own `maps/<zone>.txt` line files stay the VIEW; these two
//! feed pathfinding and ground snapping only.
//!
//! Coordinates. Three spaces exist: /loc space (log lines, wiki),
//! MAP-FILE space (the .txt view; map = (-locY, -locX, z), pinned by
//! MapViewer's brute-forced player-dot transform), and the EQEmu
//! spaces (emu = (-mapX, -mapY, z); nav = emu with y/z swapped, Detour
//! Y-up). The walk-path pipeline -- walkStartPosition, marker.pos, the
//! viewer's own waypoint drawing -- is MAP-FILE space end to end, so
//! **all public APIs here take and return MAP-FILE coordinates**. The
//! one loc-space caller (get_last_location's best-Z snap) converts at
//! its call site. zone_links.json coordinates are map-file too --
//! copy-pasteable straight from map P labels.
//!
//! Adjacency is rebuilt geometrically (shared tile-edge matching by
//! quantized endpoints) instead of decoding Detour's link/side tables --
//! fewer format assumptions, and off-mesh connections come in as plain
//! extra edges. Poly count per zone is a few thousand; the rebuild is a
//! one-time cost at load.
//!
//! Files cache under `<app_data>/emu_maps/`, fetched per zone on demand
//! (`ensure_zone`) -- ~500 zones is too much to embed, one zone is
//! ~150-500KB for both files. Missing/unfetched zones fall back to the
//! line-map pathfinder at the call site.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use crate::state::LockRecover;

// ---------------------------------------------------------------- parsing

/// why: one traversable edge out of a poly. `link` None = a real shared
/// mesh edge (a,b = the portal the funnel threads); Some(i) = a
/// zone-link HOP (teleporter pad/door) into ZoneNav::links[i] -- the
/// location changes, nothing is walked, cost is ~zero (a pad is a zone
/// line, not a journey; asked directly: "similar to zone changes...
/// instead of having a special cost").
#[derive(Debug, Clone)]
pub struct NavEdge {
    pub to: u32,
    pub a: [f32; 3],
    pub b: [f32; 3],
    pub link: Option<u32>,
}

#[derive(Debug)]
pub struct NavPoly {
    /// why: game-coordinate vertices, fan order
    pub verts: Vec<[f32; 3]>,
    pub edges: Vec<NavEdge>,
    pub center: [f32; 3],
}

#[derive(Debug)]
pub struct ZoneNav {
    pub polys: Vec<NavPoly>,
    /// why: applied links, indexed by NavEdge::link
    pub links: Vec<ZoneLink>,
    /// why: swim zone -- routes chain verified centers, never funnel-
    /// smoothed (a smoothed corner is a line nobody line-of-sight checked)
    pub swim: bool,
}

/// why: what a route is made of -- walk legs on the mesh, hop legs
/// through a link. Mirrors the zone-route hop model one level down.
#[derive(Debug)]
pub enum NavLeg {
    Walk(Vec<[f32; 3]>),
    Hop {
        at: [f32; 3],
        to: [f32; 3],
        label: String,
    },
}

#[derive(Debug)]
pub struct ZoneGeo {
    verts: Vec<[f32; 3]>,
    tris: Vec<[u32; 3]>,
    /// why: XY-cell -> triangle indices, CELL-sized buckets for best_z
    grid: HashMap<(i32, i32), Vec<u32>>,
}

const CELL: f32 = 32.0;

/// why: zones where open water is itself traversable, so straight swim
/// edges may bridge mesh gaps (see ZoneNav::bridge_gaps). Curated -- the
/// wiki has no underwater flag; Kedge Keep is the era's one fully
/// underwater zone. A land zone must NEVER get this treatment: a clear
/// line of sight across a canyon is air, not a route.
const UNDERWATER_ZONES: &[&str] = &["kedge"];

pub fn is_underwater(zone: &str) -> bool {
    UNDERWATER_ZONES.contains(&zone)
}

// ------------------------------------------------------------ zone links

/// why: traversal the mesh can't know -- teleporter pads, magic doors,
/// lifts (packs/zone_links.json). Directed, loc coords. Plane of Sky is
/// the proving case: its navmesh is 45 disconnected islands, so without
/// these edges no cross-island path can exist at all.
#[derive(Debug, serde::Deserialize, Clone)]
pub struct ZoneLink {
    pub from: [f32; 3],
    pub to: [f32; 3],
    #[allow(dead_code)]
    pub label: String,
}

#[derive(serde::Deserialize)]
struct ZoneLinksDoc {
    links: HashMap<String, Vec<ZoneLink>>,
}

const ZONE_LINKS_JSON: &str = include_str!("../../../packs/zone_links.json");

pub fn zone_links(zone: &str) -> &'static [ZoneLink] {
    static DOC: OnceLock<ZoneLinksDoc> = OnceLock::new();
    DOC.get_or_init(|| {
        serde_json::from_str(ZONE_LINKS_JSON)
            .unwrap_or_else(|e| panic!("packs/zone_links.json failed to parse: {e}"))
    })
    .links
    .get(zone)
    .map(Vec::as_slice)
    .unwrap_or(&[])
}

pub fn nav_to_game(p: [f32; 3]) -> [f32; 3] {
    // nav(emuX, z, emuY) -> map(-emuX, -emuY, z)
    [-p[0], -p[2], p[1]]
}

pub fn game_to_nav(p: [f32; 3]) -> [f32; 3] {
    [-p[0], p[2], -p[1]]
}

fn emu_to_game(p: [f32; 3]) -> [f32; 3] {
    // emu(-mapX, -mapY, z) -> map
    [-p[0], -p[1], p[2]]
}

fn rd_u32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn rd_i32(b: &[u8], o: usize) -> i32 {
    i32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn rd_f32(b: &[u8], o: usize) -> f32 {
    f32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

fn inflate(wrapped: &[u8], skip: usize, expect: usize) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(expect);
    flate2::read::ZlibDecoder::new(&wrapped[skip..])
        .read_to_end(&mut out)
        .ok()?;
    (out.len() == expect).then_some(out)
}

/// why: a tile-border edge awaiting its cross-tile partner(s). Detour
/// connects tiles by OVERLAP along the border line, not endpoint
/// equality -- adjacent tiles split the same border differently, so
/// endpoint matching drops most cross-tile adjacency (kedge: 244
/// islands, no route across the zone; every zone had orphan fragments).
struct BorderEdge {
    poly: u32,
    tile: usize,
    a: [f32; 3],
    b: [f32; 3],
}

/// Parse an `EQNAVMESH` file into a game-coordinate poly graph.
pub fn parse_nav(data: &[u8]) -> Option<ZoneNav> {
    if data.len() < 21 || &data[..9] != b"EQNAVMESH" {
        return None;
    }
    let usz = rd_u32(data, 17) as usize;
    let inner = inflate(data, 21, usz)?;
    let ntiles = rd_u32(&inner, 0) as usize;
    let mut off = 32; // u32 count + dtNavMeshParams(28)

    let mut polys: Vec<NavPoly> = Vec::new();
    let mut borders: Vec<BorderEdge> = Vec::new();
    // why: the largest walkableClimb across tiles -- the z tolerance for
    // matching border edges (a step across a tile seam is climbable
    // exactly when Detour itself would have linked it)
    let mut climb: f32 = 4.0;
    // off-mesh endpoints to connect after all polys exist
    let mut offmesh: Vec<([f32; 3], [f32; 3])> = Vec::new();

    for tile_i in 0..ntiles {
        if off + 8 > inner.len() {
            return None;
        }
        let size = rd_u32(&inner, off + 4) as usize;
        off += 8;
        let t = inner.get(off..off + size)?;
        off += size;

        // dtMeshHeader v7, 100 bytes
        if rd_u32(t, 0) != 0x444E_4156 || rd_i32(t, 4) != 7 {
            return None;
        }
        let poly_count = rd_i32(t, 24) as usize;
        let vert_count = rd_i32(t, 28) as usize;
        let off_mesh_con_count = rd_i32(t, 52) as usize;
        let off_mesh_base = rd_i32(t, 56) as usize;
        // dtMeshHeader: walkableClimb f32 at 68
        climb = climb.max(rd_f32(t, 68));

        let mut p = 100usize;
        let verts: Vec<[f32; 3]> = (0..vert_count)
            .map(|i| {
                let o = p + i * 12;
                nav_to_game([rd_f32(t, o), rd_f32(t, o + 4), rd_f32(t, o + 8)])
            })
            .collect();
        p += vert_count * 12;

        // dtPoly: u32 firstLink, u16 verts[6], u16 neis[6], u16 flags,
        // u8 vertCount, u8 areaAndtype = 32 bytes
        const POLY_SZ: usize = 32;
        // why: Detour's own adjacency, not re-derived -- neis[k] is the
        // 1-based SAME-TILE neighbor across edge k, or 0x8000|dir for a
        // tile-border edge (matched by overlap after all tiles load).
        // local[i] maps this tile's poly index to the global one so a
        // nei value can be resolved (None = skipped off-mesh stub).
        let mut local: Vec<Option<u32>> = vec![None; poly_count];
        #[allow(clippy::needless_range_loop)] // why: i also derives the byte offset
        for i in 0..poly_count {
            let o = p + i * POLY_SZ;
            let vcnt = t[o + 30] as usize;
            let ty = t[o + 31] >> 6;
            // why: off-mesh connection polys (type 1) are 2-vert stubs --
            // handled from the offMeshCons array below, not as area polys
            if ty == 1 || vcnt < 3 {
                continue;
            }
            let pverts: Vec<[f32; 3]> = (0..vcnt)
                .map(|k| {
                    let vi =
                        u16::from_le_bytes(t[o + 4 + k * 2..o + 6 + k * 2].try_into().unwrap());
                    verts[vi as usize]
                })
                .collect();
            let n = pverts.len() as f32;
            let center = [
                pverts.iter().map(|v| v[0]).sum::<f32>() / n,
                pverts.iter().map(|v| v[1]).sum::<f32>() / n,
                pverts.iter().map(|v| v[2]).sum::<f32>() / n,
            ];
            local[i] = Some(polys.len() as u32);
            polys.push(NavPoly {
                edges: Vec::new(),
                center,
                verts: pverts,
            });
        }
        // second pass: resolve neis now that every local index is known
        for i in 0..poly_count {
            let Some(gi) = local[i] else { continue };
            let o = p + i * POLY_SZ;
            let vcnt = t[o + 30] as usize;
            let pv = polys[gi as usize].verts.clone();
            for k in 0..vcnt {
                let a = pv[k];
                let b = pv[(k + 1) % pv.len()];
                let nei = u16::from_le_bytes(t[o + 16 + k * 2..o + 18 + k * 2].try_into().unwrap());
                if nei == 0 {
                    continue;
                }
                if nei & 0x8000 != 0 {
                    borders.push(BorderEdge {
                        poly: gi,
                        tile: tile_i,
                        a,
                        b,
                    });
                } else if let Some(Some(gj)) = local.get((nei - 1) as usize) {
                    // why: directed only -- the neighbor's own neis row
                    // adds the reverse (Detour keeps them symmetric)
                    polys[gi as usize].edges.push(NavEdge {
                        to: *gj,
                        a,
                        b,
                        link: None,
                    });
                }
            }
        }
        p += poly_count * POLY_SZ;
        let _ = off_mesh_base;

        // skip: links, detailMeshes, detailVerts, detailTris, bvTree --
        // sizes derive from header counts; only offMeshCons are needed
        let max_link_count = rd_i32(t, 32) as usize;
        let detail_mesh_count = rd_i32(t, 36) as usize;
        let detail_vert_count = rd_i32(t, 40) as usize;
        let detail_tri_count = rd_i32(t, 44) as usize;
        let bv_node_count = rd_i32(t, 48) as usize;
        p += max_link_count * 12; // dtLink
        p += detail_mesh_count * 12; // dtPolyDetail
        p += detail_vert_count * 12;
        p += detail_tri_count * 4;
        p += bv_node_count * 16; // dtBVNode
                                 // dtOffMeshConnection: f32 pos[6], f32 rad, u16 poly, u8 flags,
                                 // u8 side, u32 userId = 36 bytes
        for i in 0..off_mesh_con_count {
            let o = p + i * 36;
            if o + 36 > t.len() {
                break;
            }
            let a = nav_to_game([rd_f32(t, o), rd_f32(t, o + 4), rd_f32(t, o + 8)]);
            let b = nav_to_game([rd_f32(t, o + 12), rd_f32(t, o + 16), rd_f32(t, o + 20)]);
            offmesh.push((a, b));
        }
    }

    if std::env::var("EQLP_NAV_DEBUG").is_ok() {
        eprintln!(
            "nav debug: {ntiles} tiles, {} polys, {} border edges, climb {climb}",
            polys.len(),
            borders.len()
        );
        let axis_aligned = borders
            .iter()
            .filter(|e| (e.a[0] - e.b[0]).abs() < 0.05 || (e.a[1] - e.b[1]).abs() < 0.05)
            .count();
        eprintln!(
            "nav debug: {axis_aligned} axis-aligned of {}",
            borders.len()
        );
        for e in borders.iter().take(8) {
            eprintln!("  border t{} p{} a={:?} b={:?}", e.tile, e.poly, e.a, e.b);
        }
    }
    // why: cross-tile portals, Detour connectExtLinks' job -- border
    // edges lie on axis-aligned tile boundary lines; two edges from
    // different tiles on the same line connect where their intervals
    // OVERLAP (never endpoint equality), gated by walkableClimb in z so
    // stacked floors at a seam don't fuse. Portal = the overlap segment.
    {
        // bucket by (which axis is constant, that coordinate, eighth-unit)
        let mut buckets: HashMap<(u8, i64), Vec<usize>> = HashMap::new();
        for (i, e) in borders.iter().enumerate() {
            let (axis, c) = if (e.a[0] - e.b[0]).abs() < 0.05 {
                (0u8, e.a[0])
            } else if (e.a[1] - e.b[1]).abs() < 0.05 {
                (1u8, e.a[1])
            } else {
                continue; // not axis-aligned: not a tile border line
            };
            buckets
                .entry((axis, (c * 8.0).round() as i64))
                .or_default()
                .push(i);
        }
        let mut new_edges: Vec<(u32, NavEdge)> = Vec::new();
        let mut dbg = (0usize, 0usize, 0usize, 0usize); // pairs, same_tile, no_overlap, z_reject
        for ((axis, _), idxs) in &buckets {
            let va = 1 - *axis as usize; // the varying axis
            for (m, &i) in idxs.iter().enumerate() {
                for &j in &idxs[m + 1..] {
                    let (e1, e2) = (&borders[i], &borders[j]);
                    dbg.0 += 1;
                    if e1.tile == e2.tile {
                        dbg.1 += 1;
                        continue;
                    }
                    let (a1, b1) = (e1.a[va].min(e1.b[va]), e1.a[va].max(e1.b[va]));
                    let (a2, b2) = (e2.a[va].min(e2.b[va]), e2.a[va].max(e2.b[va]));
                    let (t0, t1) = (a1.max(a2), b1.min(b2));
                    if t1 - t0 < 0.05 {
                        dbg.2 += 1;
                        continue;
                    }
                    // z at the overlap ends, interpolated along each edge
                    let z_at = |e: &BorderEdge, t: f32| -> f32 {
                        let (ta, tb) = (e.a[va], e.b[va]);
                        if (tb - ta).abs() < 1e-6 {
                            return e.a[2];
                        }
                        let f = (t - ta) / (tb - ta);
                        e.a[2] + (e.b[2] - e.a[2]) * f
                    };
                    if (z_at(e1, t0) - z_at(e2, t0)).abs() > climb
                        || (z_at(e1, t1) - z_at(e2, t1)).abs() > climb
                    {
                        dbg.3 += 1;
                        continue;
                    }
                    let portal = |e: &BorderEdge| -> ([f32; 3], [f32; 3]) {
                        let mut pa = e.a;
                        let mut pb = e.a;
                        pa[va] = t0;
                        pa[2] = z_at(e, t0);
                        pb[va] = t1;
                        pb[2] = z_at(e, t1);
                        (pa, pb)
                    };
                    let (p1a, p1b) = portal(e1);
                    let (p2a, p2b) = portal(e2);
                    new_edges.push((
                        e1.poly,
                        NavEdge {
                            to: e2.poly,
                            a: p1a,
                            b: p1b,
                            link: None,
                        },
                    ));
                    new_edges.push((
                        e2.poly,
                        NavEdge {
                            to: e1.poly,
                            a: p2a,
                            b: p2b,
                            link: None,
                        },
                    ));
                }
            }
        }
        if std::env::var("EQLP_NAV_DEBUG").is_ok() {
            eprintln!(
                "nav debug: {} buckets, pairs={} same_tile={} no_overlap={} z_reject={} connected={}",
                buckets.len(),
                dbg.0,
                dbg.1,
                dbg.2,
                dbg.3,
                new_edges.len() / 2
            );
        }
        for (from, e) in new_edges {
            polys[from as usize].edges.push(e);
        }
    }

    // off-mesh connections: link the polys whose centers sit nearest each endpoint
    for (a, b) in offmesh {
        let near = |pt: [f32; 3], polys: &[NavPoly]| -> Option<u32> {
            polys
                .iter()
                .enumerate()
                .min_by(|(_, p1), (_, p2)| {
                    dist2(p1.center, pt)
                        .partial_cmp(&dist2(p2.center, pt))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|(i, _)| i as u32)
        };
        if let (Some(pa), Some(pb)) = (near(a, &polys), near(b, &polys)) {
            if pa != pb {
                polys[pa as usize].edges.push(NavEdge {
                    to: pb,
                    a,
                    b,
                    link: None,
                });
                polys[pb as usize].edges.push(NavEdge {
                    to: pa,
                    a: b,
                    b: a,
                    link: None,
                });
            }
        }
    }

    (!polys.is_empty()).then_some(ZoneNav {
        polys,
        links: Vec::new(),
        swim: false,
    })
}

/// Parse an EQEmu v2 `base/*.map` collision mesh into a best-Z index.
pub fn parse_map(data: &[u8]) -> Option<ZoneGeo> {
    if data.len() < 12 || rd_u32(data, 0) != 0x0200_0000 {
        return None;
    }
    let usz = rd_u32(data, 8) as usize;
    let inner = inflate(data, 12, usz)?;
    let vert_count = rd_u32(&inner, 0) as usize;
    let ind_count = rd_u32(&inner, 4) as usize;
    let mut off = 40; // 9 u32 counts + f32 units_per_vertex
    let verts: Vec<[f32; 3]> = (0..vert_count)
        .map(|i| {
            let o = off + i * 12;
            emu_to_game([
                rd_f32(&inner, o),
                rd_f32(&inner, o + 4),
                rd_f32(&inner, o + 8),
            ])
        })
        .collect();
    off += vert_count * 12;
    let tris: Vec<[u32; 3]> = (0..ind_count / 3)
        .map(|i| {
            let o = off + i * 12;
            [
                rd_u32(&inner, o),
                rd_u32(&inner, o + 4),
                rd_u32(&inner, o + 8),
            ]
        })
        .collect();

    let mut grid: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
    for (ti, t) in tris.iter().enumerate() {
        let (mut minx, mut maxx) = (f32::MAX, f32::MIN);
        let (mut miny, mut maxy) = (f32::MAX, f32::MIN);
        for &vi in t {
            let v = verts[vi as usize];
            minx = minx.min(v[0]);
            maxx = maxx.max(v[0]);
            miny = miny.min(v[1]);
            maxy = maxy.max(v[1]);
        }
        let (cx0, cx1) = ((minx / CELL).floor() as i32, (maxx / CELL).floor() as i32);
        let (cy0, cy1) = ((miny / CELL).floor() as i32, (maxy / CELL).floor() as i32);
        for cx in cx0..=cx1 {
            for cy in cy0..=cy1 {
                grid.entry((cx, cy)).or_default().push(ti as u32);
            }
        }
    }
    Some(ZoneGeo { verts, tris, grid })
}

fn dist2(a: [f32; 3], b: [f32; 3]) -> f32 {
    let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    d[0] * d[0] + d[1] * d[1] + d[2] * d[2]
}

// ---------------------------------------------------------------- best_z

impl ZoneGeo {
    /// Ground height at game (x, y): the triangle surface nearest
    /// `z_hint` among those containing the point in XY projection.
    /// None when no geometry covers the point (off the world).
    pub fn best_z(&self, x: f32, y: f32, z_hint: f32) -> Option<f32> {
        let cell = ((x / CELL).floor() as i32, (y / CELL).floor() as i32);
        let mut best: Option<f32> = None;
        for &ti in self.grid.get(&cell)? {
            let [a, b, c] = self.tris[ti as usize];
            let (va, vb, vc) = (
                self.verts[a as usize],
                self.verts[b as usize],
                self.verts[c as usize],
            );
            // 2D barycentric containment in XY
            let d = (vb[1] - vc[1]) * (va[0] - vc[0]) + (vc[0] - vb[0]) * (va[1] - vc[1]);
            if d.abs() < 1e-9 {
                continue;
            }
            let l1 = ((vb[1] - vc[1]) * (x - vc[0]) + (vc[0] - vb[0]) * (y - vc[1])) / d;
            let l2 = ((vc[1] - va[1]) * (x - vc[0]) + (va[0] - vc[0]) * (y - vc[1])) / d;
            let l3 = 1.0 - l1 - l2;
            if !(-0.001..=1.001).contains(&l1)
                || !(-0.001..=1.001).contains(&l2)
                || !(-0.001..=1.001).contains(&l3)
            {
                continue;
            }
            let z = l1 * va[2] + l2 * vb[2] + l3 * vc[2];
            if best.is_none_or(|bz| (z - z_hint).abs() < (bz - z_hint).abs()) {
                best = Some(z);
            }
        }
        best
    }

    /// why: 3D segment vs collision mesh -- true when nothing blocks the
    /// straight line (open water for bridge_gaps' swim edges). Candidate
    /// triangles come from the XY grid cells the segment crosses.
    pub fn los_clear(&self, a: [f32; 3], b: [f32; 3]) -> bool {
        let mut cells: Vec<(i32, i32)> = Vec::new();
        let len_xy = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
        let steps = (len_xy / (CELL * 0.5)).ceil().max(1.0) as usize;
        for s in 0..=steps {
            let f = s as f32 / steps as f32;
            let x = a[0] + (b[0] - a[0]) * f;
            let y = a[1] + (b[1] - a[1]) * f;
            // why: 3x3 neighborhood -- a segment grazing a cell border
            // must still see the neighbor cell's triangles
            let (ci, cj) = ((x / CELL).floor() as i32, (y / CELL).floor() as i32);
            for di in -1..=1 {
                for dj in -1..=1 {
                    let c = (ci + di, cj + dj);
                    if !cells.contains(&c) {
                        cells.push(c);
                    }
                }
            }
        }
        let dir = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let mut seen: std::collections::HashSet<u32> = std::collections::HashSet::new();
        for c in cells {
            let Some(tris) = self.grid.get(&c) else {
                continue;
            };
            for &ti in tris {
                if !seen.insert(ti) {
                    continue;
                }
                let [i, j, k] = self.tris[ti as usize];
                if seg_hits_tri(
                    a,
                    dir,
                    self.verts[i as usize],
                    self.verts[j as usize],
                    self.verts[k as usize],
                ) {
                    return false;
                }
            }
        }
        true
    }
}

impl ZoneGeo {
    /// why: probe-only strict audit -- every triangle, no grid, so a
    /// route can be checked by a method independent of los_clear's own
    /// cell sampling. Slow on purpose; never call from routing.
    pub fn segment_hits_any_tri(&self, a: [f32; 3], b: [f32; 3]) -> bool {
        let dir = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        self.tris.iter().any(|&[i, j, k]| {
            seg_hits_tri(
                a,
                dir,
                self.verts[i as usize],
                self.verts[j as usize],
                self.verts[k as usize],
            )
        })
    }
}

/// why: Moller-Trumbore, hit only counts strictly inside the segment
/// (t in (0.001, 0.999) -- endpoints sit ON geometry by construction)
fn seg_hits_tri(orig: [f32; 3], dir: [f32; 3], v0: [f32; 3], v1: [f32; 3], v2: [f32; 3]) -> bool {
    let sub = |p: [f32; 3], q: [f32; 3]| [p[0] - q[0], p[1] - q[1], p[2] - q[2]];
    let cross = |p: [f32; 3], q: [f32; 3]| {
        [
            p[1] * q[2] - p[2] * q[1],
            p[2] * q[0] - p[0] * q[2],
            p[0] * q[1] - p[1] * q[0],
        ]
    };
    let dot = |p: [f32; 3], q: [f32; 3]| p[0] * q[0] + p[1] * q[1] + p[2] * q[2];
    let e1 = sub(v1, v0);
    let e2 = sub(v2, v0);
    let h = cross(dir, e2);
    let det = dot(e1, h);
    if det.abs() < 1e-9 {
        return false;
    }
    let inv = 1.0 / det;
    let s = sub(orig, v0);
    let u = dot(s, h) * inv;
    if !(0.0..=1.0).contains(&u) {
        return false;
    }
    let q = cross(s, e1);
    let v = dot(dir, q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return false;
    }
    let t = dot(e2, q) * inv;
    (0.001..0.999).contains(&t)
}

// ---------------------------------------------------------------- pathfinding

impl ZoneNav {
    /// why: inject directed traversal links (teleporters/doors/lifts)
    /// between the polys nearest each endpoint. Reverse edges are
    /// deliberately not added: a one-way port is one-way.
    pub fn apply_links(&mut self, links: &[ZoneLink]) {
        for l in links {
            let (Some(pa), Some(pb)) = (self.nearest_poly(l.from), self.nearest_poly(l.to)) else {
                continue;
            };
            if pa != pb {
                let li = self.links.len() as u32;
                self.links.push(l.clone());
                self.polys[pa as usize].edges.push(NavEdge {
                    to: pb,
                    a: l.from,
                    b: l.from,
                    link: Some(li),
                });
            }
        }
    }

    /// why: pub wrapper for probes only -- see nav_components_check
    pub fn nearest_poly_pub(&self, p: [f32; 3]) -> Option<u32> {
        self.nearest_poly(p)
    }

    /// why: open water as graph nodes -- a coarse 3D grid over the mesh,
    /// each point kept only if a SHORT line of sight reaches a nearby
    /// poly or node. Short hops chain through shafts and rooms where one
    /// long line never clears (kedge's top level: 7 clear lines among
    /// 358k poly pairs, all ~615 units -- a needle no sampling finds).
    /// Nodes become single-vertex polys, so routing/funnel/endpoint
    /// binding need no special case; a void cluster outside the zone
    /// only ever sees itself and stays unreachable.
    fn add_water_nodes(&mut self, geo: &ZoneGeo, z_lift: f32) {
        const STEP: f32 = 24.0;
        // why: > STEP*sqrt(3) so diagonal grid neighbors can link;
        // corridor-scale so a hop never crosses a room
        const REACH: f32 = 42.0;
        // why: a node that sees no poly itself may still sit in a shaft
        // between two that do -- allowed a few hops from one, bounded so
        // outside-the-wall points at an opening can't grow a highway
        const MAX_HOPS_FROM_MESH: u8 = 3;
        let n = self.polys.len();
        if n == 0 {
            return;
        }
        let mut lo = [f32::MAX; 3];
        let mut hi = [f32::MIN; 3];
        for p in &self.polys {
            for k in 0..3 {
                lo[k] = lo[k].min(p.center[k]);
                hi[k] = hi[k].max(p.center[k]);
            }
        }
        let cell = |v: f32| (v / REACH).floor() as i32;
        let mut poly_cells: HashMap<(i32, i32, i32), Vec<u32>> = HashMap::new();
        for (i, p) in self.polys.iter().enumerate() {
            poly_cells
                .entry((cell(p.center[0]), cell(p.center[1]), cell(p.center[2])))
                .or_default()
                .push(i as u32);
        }
        let mut nodes: Vec<[f32; 3]> = Vec::new();
        let mut x = lo[0] - STEP;
        while x <= hi[0] + STEP {
            let mut y = lo[1] - STEP;
            while y <= hi[1] + STEP {
                let mut z = lo[2];
                while z <= hi[2] + STEP {
                    nodes.push([x, y, z]);
                    z += STEP;
                }
                y += STEP;
            }
            x += STEP;
        }
        let mut node_cells: HashMap<(i32, i32, i32), Vec<u32>> = HashMap::new();
        for (i, p) in nodes.iter().enumerate() {
            node_cells
                .entry((cell(p[0]), cell(p[1]), cell(p[2])))
                .or_default()
                .push(i as u32);
        }
        let reach2 = REACH * REACH;
        // edges per node: (to poly idx, ...) and (to node idx, ...)
        let mut node_polys: Vec<Vec<u32>> = vec![Vec::new(); nodes.len()];
        let mut node_nodes: Vec<Vec<u32>> = vec![Vec::new(); nodes.len()];
        let mut tests = 0usize;
        for (ni, p) in nodes.iter().enumerate() {
            let (cx, cy, cz) = (cell(p[0]), cell(p[1]), cell(p[2]));
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        let key = (cx + dx, cy + dy, cz + dz);
                        if let Some(ps) = poly_cells.get(&key) {
                            for &pi in ps {
                                let c = self.polys[pi as usize].center;
                                if dist2(*p, c) > reach2 {
                                    continue;
                                }
                                let mut lc = c;
                                lc[2] += z_lift;
                                tests += 1;
                                if geo.los_clear(*p, lc) {
                                    node_polys[ni].push(pi);
                                }
                            }
                        }
                        if let Some(ns) = node_cells.get(&key) {
                            for &nj in ns {
                                if nj as usize <= ni || dist2(*p, nodes[nj as usize]) > reach2 {
                                    continue;
                                }
                                tests += 1;
                                if geo.los_clear(*p, nodes[nj as usize]) {
                                    node_nodes[ni].push(nj);
                                }
                            }
                        }
                    }
                }
            }
        }
        // why: a node lives only if it can SEE a real mesh poly -- points
        // outside the walls see each other freely (no geometry between
        // them) and formed a highway around the keep that the zone-line
        // opening let routes escape into ("swimming straight down outside
        // the map"). Node-node links then only ever join inside points.
        // hop distance from any poly-seeing node, BFS over node links
        let mut hops: Vec<u8> = vec![u8::MAX; nodes.len()];
        let mut q = std::collections::VecDeque::new();
        for ni in 0..nodes.len() {
            if !node_polys[ni].is_empty() {
                hops[ni] = 0;
                q.push_back(ni);
            }
        }
        let mut adj: Vec<Vec<u32>> = vec![Vec::new(); nodes.len()];
        for ni in 0..nodes.len() {
            for &nj in &node_nodes[ni] {
                adj[ni].push(nj);
                adj[nj as usize].push(ni as u32);
            }
        }
        while let Some(i) = q.pop_front() {
            let h = hops[i];
            if h >= MAX_HOPS_FROM_MESH {
                continue;
            }
            for &j in &adj[i] {
                if hops[j as usize] == u8::MAX {
                    hops[j as usize] = h + 1;
                    q.push_back(j as usize);
                }
            }
        }
        let mut keep: Vec<Option<u32>> = vec![None; nodes.len()];
        for (ni, p) in nodes.iter().enumerate() {
            if hops[ni] <= MAX_HOPS_FROM_MESH {
                keep[ni] = Some(self.polys.len() as u32);
                self.polys.push(NavPoly {
                    verts: vec![*p],
                    edges: Vec::new(),
                    center: *p,
                });
            }
        }
        let mut edges = 0usize;
        for ni in 0..nodes.len() {
            let Some(gi) = keep[ni] else { continue };
            let np = nodes[ni];
            for &pi in &node_polys[ni] {
                let c = self.polys[pi as usize].center;
                self.polys[gi as usize].edges.push(NavEdge {
                    to: pi,
                    a: c,
                    b: c,
                    link: None,
                });
                self.polys[pi as usize].edges.push(NavEdge {
                    to: gi,
                    a: np,
                    b: np,
                    link: None,
                });
                edges += 1;
            }
            for &nj in &node_nodes[ni] {
                let Some(gj) = keep[nj as usize] else {
                    continue;
                };
                let q = nodes[nj as usize];
                self.polys[gi as usize].edges.push(NavEdge {
                    to: gj,
                    a: q,
                    b: q,
                    link: None,
                });
                self.polys[gj as usize].edges.push(NavEdge {
                    to: gi,
                    a: np,
                    b: np,
                    link: None,
                });
                edges += 1;
            }
        }
        if std::env::var("EQLP_NAV_DEBUG").is_ok() {
            eprintln!(
                "water nodes: {} grid points, {} kept, {} edges, {} LOS tests",
                nodes.len(),
                self.polys.len() - n,
                edges,
                tests
            );
        }
    }

    /// why: underwater zones only (see UNDERWATER_ZONES). EQEmu's mesh
    /// for a swim zone is floor patches with NO swim connectivity at all
    /// (kedge: 243 components, no off-mesh cons, one area type --
    /// audited, not assumed), so open water itself must be the bridge:
    /// straight swim edges between components wherever the collision
    /// mesh shows a clear line. Both portal endpoints are the far
    /// center, so the funnel threads the exact swim point.
    pub fn bridge_gaps(&mut self, geo: &ZoneGeo) {
        // why: doorway-scale only -- a swim zone is a building of
        // corridors; the mesh is the truth and a bridge is a patch for a
        // real gap, never a room-to-room shortcut (nodes handle shafts)
        const MAX_DIST: f32 = 80.0;
        const Z_LIFT: f32 = 2.0;
        self.add_water_nodes(geo, Z_LIFT);

        // components over current adjacency
        let n = self.polys.len();
        let mut comp = vec![u32::MAX; n];
        let mut c = 0u32;
        for s in 0..n {
            if comp[s] != u32::MAX {
                continue;
            }
            let mut q = std::collections::VecDeque::from([s]);
            comp[s] = c;
            while let Some(i) = q.pop_front() {
                for e in &self.polys[i].edges {
                    let t = e.to as usize;
                    if comp[t] == u32::MAX {
                        comp[t] = c;
                        q.push_back(t);
                    }
                }
            }
            c += 1;
        }
        if c <= 1 {
            return;
        }

        // why: greedy union-find over ALL candidate pairs sorted by
        // distance -- a successful bridge merges two sets, so every later
        // pair between them is skipped without a line-of-sight test.
        // The per-component-pair scheme before this tested 2.4M lines
        // on kedge (243 sets -> 29k set pairs); this needs a fraction.
        // Per set pair, tries are capped so a hopeless pair (two sets
        // wall-separated everywhere) can't scan the whole zone.
        const MAX_TRIES: usize = 400;
        let mut cands: Vec<(f32, u32, u32)> = Vec::new();
        for i in 0..n {
            for j in (i + 1)..n {
                if comp[i] == comp[j] {
                    continue;
                }
                let d2 = dist2(self.polys[i].center, self.polys[j].center);
                if d2 <= MAX_DIST * MAX_DIST {
                    cands.push((d2, i as u32, j as u32));
                }
            }
        }
        cands.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(Ordering::Equal));

        let mut parent: Vec<u32> = (0..c).collect();
        fn find(parent: &mut [u32], x: u32) -> u32 {
            let mut r = x;
            while parent[r as usize] != r {
                r = parent[r as usize];
            }
            let mut cur = x;
            while parent[cur as usize] != r {
                let next = parent[cur as usize];
                parent[cur as usize] = r;
                cur = next;
            }
            r
        }
        let lift = |mut p: [f32; 3]| {
            p[2] += Z_LIFT;
            p
        };
        let mut tries: HashMap<(u32, u32), usize> = HashMap::new();
        let mut sets_left = c;
        let mut dbg = (0usize, 0usize); // LOS tests, connected
        for (_, i, j) in cands {
            if sets_left <= 1 {
                break;
            }
            let (ra, rb) = (
                find(&mut parent, comp[i as usize]),
                find(&mut parent, comp[j as usize]),
            );
            if ra == rb {
                continue;
            }
            let key = (ra.min(rb), ra.max(rb));
            let t = tries.entry(key).or_insert(0);
            if *t >= MAX_TRIES {
                continue;
            }
            *t += 1;
            dbg.0 += 1;
            let a = self.polys[i as usize].center;
            let b = self.polys[j as usize].center;
            if !geo.los_clear(lift(a), lift(b)) {
                continue;
            }
            dbg.1 += 1;
            self.polys[i as usize].edges.push(NavEdge {
                to: j,
                a: b,
                b,
                link: None,
            });
            self.polys[j as usize].edges.push(NavEdge {
                to: i,
                a,
                b: a,
                link: None,
            });
            parent[ra as usize] = rb;
            sets_left -= 1;
        }
        if std::env::var("EQLP_NAV_DEBUG").is_ok() {
            eprintln!(
                "bridge debug: {c} components -> {sets_left}, {} LOS tests, {} connected",
                dbg.0, dbg.1
            );
        }
    }

    /// why: nearest by the same weighting, but the first one with a
    /// clear line from `p` (bounded scan); falls back to plain nearest
    fn nearest_poly_los(&self, p: [f32; 3], geo: &ZoneGeo) -> Option<u32> {
        const TRIES: usize = 300;
        let mut order: Vec<(f32, u32)> = self
            .polys
            .iter()
            .enumerate()
            .map(|(i, poly)| (weighted(poly.center, p), i as u32))
            .collect();
        order.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        order
            .iter()
            .take(TRIES)
            .map(|&(_, i)| i)
            .find(|&i| {
                let mut c = self.polys[i as usize].center;
                c[2] += 2.0;
                geo.los_clear(p, c)
            })
            .or_else(|| order.first().map(|&(_, i)| i))
    }

    fn nearest_poly(&self, p: [f32; 3]) -> Option<u32> {
        self.polys
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                // why: XY distance dominates, height tie-breaks at 1:4 --
                // "nearest walkable spot under/near me", not a poly on a
                // floor 3 levels up that happens to be XY-closer
                let da = weighted(a.center, p);
                let db = weighted(b.center, p);
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(i, _)| i as u32)
    }

    /// A* over poly adjacency + per-walk-leg funnel smoothing. Map-file
    /// coords in and out. A link edge costs ~nothing (a pad is a zone
    /// line, not a journey) and splits the route into separate legs.
    /// None when either endpoint has no nearby poly or no route exists.
    pub fn find_route(
        &self,
        from: [f32; 3],
        to: [f32; 3],
        geo: Option<&ZoneGeo>,
    ) -> Option<Vec<NavLeg>> {
        /// why: not literally 0.0 -- a tiny positive cost keeps A*
        /// admissible-ish and route lengths finite under link cycles
        const LINK_COST: f32 = 1.0;
        // why: swim zones bind each endpoint to the nearest poly/node it
        // can actually see -- the plain nearest may sit behind a wall,
        // and that first/last leg is never otherwise checked
        let bind = |p: [f32; 3]| -> Option<u32> {
            match (self.swim, geo) {
                (true, Some(g)) => self.nearest_poly_los(p, g),
                _ => self.nearest_poly(p),
            }
        };
        let start = bind(from)?;
        let goal = bind(to)?;
        if start == goal {
            return Some(vec![NavLeg::Walk(ground_hug(vec![from, to], geo))]);
        }

        #[derive(PartialEq)]
        struct Node(f32, u32);
        impl Eq for Node {}
        impl Ord for Node {
            fn cmp(&self, o: &Self) -> std::cmp::Ordering {
                o.0.partial_cmp(&self.0)
                    .unwrap_or(std::cmp::Ordering::Equal)
            }
        }
        impl PartialOrd for Node {
            fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
                Some(self.cmp(o))
            }
        }

        // why: h=0 (Dijkstra) -- links make straight-line distance
        // inadmissible (a far island can be one near-zero-cost hop
        // away), and a few thousand polys is microseconds anyway
        let mut g: HashMap<u32, f32> = HashMap::from([(start, 0.0)]);
        let mut came: HashMap<u32, (u32, NavEdge)> = HashMap::new();
        let mut heap = BinaryHeap::from([Node(0.0, start)]);
        while let Some(Node(_, cur)) = heap.pop() {
            if cur == goal {
                break;
            }
            let gc = g[&cur];
            for e in &self.polys[cur as usize].edges {
                let step = if e.link.is_some() {
                    LINK_COST
                } else {
                    dist2(
                        self.polys[cur as usize].center,
                        self.polys[e.to as usize].center,
                    )
                    .sqrt()
                };
                let ng = gc + step;
                if g.get(&e.to).is_none_or(|&old| ng < old) {
                    g.insert(e.to, ng);
                    came.insert(e.to, (cur, e.clone()));
                    heap.push(Node(ng, e.to));
                }
            }
        }
        came.contains_key(&goal).then_some(())?;

        // edge chain goal -> start, reversed, then split into legs at hops
        let mut chain: Vec<NavEdge> = Vec::new();
        let mut cur = goal;
        while cur != start {
            let (prev, e) = came[&cur].clone();
            chain.push(e);
            cur = prev;
        }
        chain.reverse();

        let mut legs: Vec<NavLeg> = Vec::new();
        let mut seg_start = from;
        let mut portals: Vec<([f32; 3], [f32; 3])> = Vec::new();
        // why: swim zones keep the center chain -- every consecutive
        // pair is either mesh-adjacent (convex polys: the center-to-
        // center line crosses their shared edge) or a line-of-sight
        // verified swim hop; funnel smoothing would invent unverified
        // straight lines through walls (reported: "straight through
        // the walls on Kedge")
        // why: swim -- the chain begins ON the mesh (the zone-line label
        // a fallback start uses sits outside the tunnel mouth, so a
        // line from it "comes in from outside"); a mesh-poly waypoint
        // floats 2u above its center
        let swim = self.swim;
        let lifted = |i: u32| -> [f32; 3] {
            let mut c = self.polys[i as usize].center;
            if self.polys[i as usize].verts.len() > 1 {
                c[2] += 2.0;
            }
            c
        };
        let mut chain_pts: Vec<[f32; 3]> = if swim {
            vec![lifted(start)]
        } else {
            vec![from]
        };
        let close_leg = |seg_start: [f32; 3],
                         end: [f32; 3],
                         portals: &[([f32; 3], [f32; 3])],
                         chain_pts: &mut Vec<[f32; 3]>|
         -> Vec<[f32; 3]> {
            if swim {
                let mut pts = std::mem::take(chain_pts);
                if pts.last() != Some(&end) {
                    pts.push(end);
                }
                pts.dedup();
                pts
            } else {
                ground_hug(funnel(seg_start, end, portals), geo)
            }
        };
        // why: portal orientation needs the travel direction, so track
        // which poly each edge leaves from as the chain walks
        let mut cur_poly = start;
        for e in &chain {
            if let Some(li) = e.link {
                let l = &self.links[li as usize];
                // close the walk leg AT the pad, then the hop itself
                legs.push(NavLeg::Walk(close_leg(
                    seg_start,
                    l.from,
                    &portals,
                    &mut chain_pts,
                )));
                portals.clear();
                legs.push(NavLeg::Hop {
                    at: l.from,
                    to: l.to,
                    label: l.label.clone(),
                });
                seg_start = l.to;
                chain_pts.push(l.to);
            } else {
                portals.push(orient_portal(
                    e.a,
                    e.b,
                    self.polys[cur_poly as usize].center,
                    self.polys[e.to as usize].center,
                ));
                if swim {
                    // why: a mesh-to-mesh hop passes explicitly through
                    // the shared edge's midpoint (the doorway) -- more,
                    // shorter lines that stay inside both polys; a swim
                    // hop's portal is already the far point itself
                    let is_mesh_hop = e.a != e.b;
                    if is_mesh_hop {
                        chain_pts.push([
                            (e.a[0] + e.b[0]) * 0.5,
                            (e.a[1] + e.b[1]) * 0.5,
                            (e.a[2] + e.b[2]) * 0.5 + 2.0,
                        ]);
                    }
                    chain_pts.push(lifted(e.to));
                }
            }
            cur_poly = e.to;
        }
        legs.push(NavLeg::Walk(close_leg(
            seg_start,
            to,
            &portals,
            &mut chain_pts,
        )));
        // why: swim -- every output segment is re-verified against the
        // collision mesh; a route that would cross a wall is refused,
        // never drawn ("super verify it doesn't cross any boundaries")
        if swim {
            if let Some(g) = geo {
                let mut bad = 0usize;
                for leg in &legs {
                    if let NavLeg::Walk(pts) = leg {
                        for w in pts.windows(2) {
                            if !g.los_clear(w[0], w[1]) {
                                bad += 1;
                            }
                        }
                    }
                }
                if std::env::var("EQLP_NAV_DEBUG").is_ok() {
                    eprintln!("swim route verify: {bad} unverified segments");
                }
                if bad > 0 {
                    return None;
                }
            }
        }
        Some(legs)
    }

    /// why: flat waypoint list for callers that only draw one line --
    /// hop discontinuities just become straight segments
    pub fn find_path(&self, from: [f32; 3], to: [f32; 3]) -> Option<Vec<[f32; 3]>> {
        let legs = self.find_route(from, to, None)?;
        let mut out: Vec<[f32; 3]> = Vec::new();
        for leg in &legs {
            match leg {
                NavLeg::Walk(w) => {
                    for p in w {
                        if out.last() != Some(p) {
                            out.push(*p);
                        }
                    }
                }
                NavLeg::Hop { at, to, .. } => {
                    if out.last() != Some(at) {
                        out.push(*at);
                    }
                    out.push(*to);
                }
            }
        }
        Some(out)
    }
}

fn weighted(a: [f32; 3], b: [f32; 3]) -> f32 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    let dz = (a[2] - b[2]) * 4.0;
    dx * dx + dy * dy + dz * dz
}

/// why: the funnel only emits waypoints at CORNERS -- an open-terrain
/// route that's straight in XY comes back as two points, and the drawn
/// segment cuts straight through every hill between them (reported
/// live: Feerrott, "just a straight line" through geometry). Subdivide
/// each leg and snap every sample to the collision mesh's surface so
/// the route hugs the ground; without geo (zone .map not cached) the
/// lerped heights stand.
const HUG_STEP: f32 = 25.0;

fn ground_hug(pts: Vec<[f32; 3]>, geo: Option<&ZoneGeo>) -> Vec<[f32; 3]> {
    let Some(geo) = geo else { return pts };
    let mut out: Vec<[f32; 3]> = Vec::with_capacity(pts.len() * 4);
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let dx = b[0] - a[0];
        let dy = b[1] - a[1];
        let steps = ((dx * dx + dy * dy).sqrt() / HUG_STEP).ceil().max(1.0) as usize;
        for i in 0..steps {
            let t = i as f32 / steps as f32;
            let x = a[0] + dx * t;
            let y = a[1] + dy * t;
            let zl = a[2] + (b[2] - a[2]) * t;
            let z = geo.best_z(x, y, zl).unwrap_or(zl);
            out.push([x, y, z]);
        }
    }
    if let Some(last) = pts.last() {
        let z = geo.best_z(last[0], last[1], last[2]).unwrap_or(last[2]);
        out.push([last[0], last[1], z]);
    }
    out
}

/// Canonical simple-stupid-funnel over the XY projection (Detour's own
/// shape). Portals arrive PRE-ORIENTED (left, right) relative to travel
/// -- orientation decided once per portal from the poly-center travel
/// direction in orient_portal, never re-decided per apex: the first
/// version of this re-swapped each portal against the current apex,
/// which classified every corridor as "already inside" and emitted ZERO
/// corners -- routes rendered as one straight line through walls
/// (reported live in Feerrott; reproduced in blackburrow's tunnels:
/// 131 wall crossings on a straight "path"). Z rides along from
/// whichever portal endpoint each emitted corner comes from.
fn funnel(from: [f32; 3], to: [f32; 3], portals: &[([f32; 3], [f32; 3])]) -> Vec<[f32; 3]> {
    fn triarea2(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> f32 {
        (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
    }
    let veq = |a: [f32; 3], b: [f32; 3]| (a[0] - b[0]).abs() < 1e-4 && (a[1] - b[1]).abs() < 1e-4;

    // portals plus a zero-width goal portal
    let mut ps: Vec<([f32; 3], [f32; 3])> = Vec::with_capacity(portals.len() + 1);
    ps.extend_from_slice(portals);
    ps.push((to, to));

    let mut path = vec![from];
    let mut apex = from;
    let (mut left, mut right) = (ps[0].0, ps[0].1);
    let (mut li, mut ri) = (0usize, 0usize);
    let mut i = 1;
    while i < ps.len() {
        let (l, r) = ps[i];
        // tighten the right side
        if triarea2(apex, right, r) <= 0.0 {
            if veq(apex, right) || triarea2(apex, left, r) > 0.0 {
                right = r;
                ri = i;
            } else {
                // right crossed left: left is a corner
                if !veq(apex, left) {
                    path.push(left);
                }
                apex = left;
                let restart = li;
                left = apex;
                right = apex;
                ri = restart;
                i = restart + 1;
                continue;
            }
        }
        // tighten the left side
        if triarea2(apex, left, l) >= 0.0 {
            if veq(apex, left) || triarea2(apex, right, l) < 0.0 {
                left = l;
                li = i;
            } else {
                if !veq(apex, right) {
                    path.push(right);
                }
                apex = right;
                let restart = ri;
                left = apex;
                right = apex;
                li = restart;
                i = restart + 1;
                continue;
            }
        }
        i += 1;
    }
    if path.last().is_none_or(|p| !veq(*p, to)) {
        path.push(to);
    }
    path
}

/// why: portal orientation, decided ONCE from travel direction (current
/// poly center -> next poly center): the endpoint to the left of travel
/// is `left`. See funnel's own doc for what deciding per-apex broke.
fn orient_portal(
    a: [f32; 3],
    b: [f32; 3],
    from_center: [f32; 3],
    to_center: [f32; 3],
) -> ([f32; 3], [f32; 3]) {
    let dir = [to_center[0] - from_center[0], to_center[1] - from_center[1]];
    let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
    let rel = [a[0] - mid[0], a[1] - mid[1]];
    // why: map-file XY is orientation-REVERSED vs loc space (the
    // loc->map transform's determinant is negative), so "left of
    // travel" is the negative cross side here -- verified empirically:
    // the positive-side choice bent routes outward through walls
    if dir[0] * rel[1] - dir[1] * rel[0] < 0.0 {
        (a, b)
    } else {
        (b, a)
    }
}

// ---------------------------------------------------------------- cache + fetch

const RAW_BASE: &str = "https://raw.githubusercontent.com/EQEmu/maps/master";

pub fn cache_dir(app_data: &Path) -> PathBuf {
    app_data.join("emu_maps")
}

type NavCache = Mutex<HashMap<String, Option<Arc<ZoneNav>>>>;
type GeoCache = Mutex<HashMap<String, Option<Arc<ZoneGeo>>>>;

fn nav_cache() -> &'static NavCache {
    static C: OnceLock<NavCache> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}
fn geo_cache() -> &'static GeoCache {
    static C: OnceLock<GeoCache> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

/// why: disk-cache-only load, never network -- callers on the command
/// path must not block on a download. `None` is cached too, so a zone
/// with no file (or a bad parse) is one disk probe, not one per call.
pub fn load_nav(app_data: &Path, zone: &str) -> Option<Arc<ZoneNav>> {
    if let Some(hit) = nav_cache().lock_recover().get(zone) {
        return hit.clone();
    }
    let parsed = std::fs::read(cache_dir(app_data).join(format!("{zone}.nav")))
        .ok()
        .and_then(|b| parse_nav(&b))
        .map(|mut nav| {
            // why: pack-declared teleporter/door edges -- see zone_links
            nav.apply_links(zone_links(zone));
            // why: swim zones need open water as edges -- see bridge_gaps
            if is_underwater(zone) {
                nav.swim = true;
                if let Some(geo) = load_geo(app_data, zone) {
                    nav.bridge_gaps(&geo);
                }
            }
            Arc::new(nav)
        });
    nav_cache()
        .lock_recover()
        .insert(zone.to_string(), parsed.clone());
    parsed
}

pub fn load_geo(app_data: &Path, zone: &str) -> Option<Arc<ZoneGeo>> {
    if let Some(hit) = geo_cache().lock_recover().get(zone) {
        return hit.clone();
    }
    let parsed = std::fs::read(cache_dir(app_data).join(format!("{zone}.map")))
        .ok()
        .and_then(|b| parse_map(&b))
        .map(Arc::new);
    geo_cache()
        .lock_recover()
        .insert(zone.to_string(), parsed.clone());
    parsed
}

/// Download a zone's nav+map into the cache if absent. Async, called
/// from its own command when the Maps view opens a zone; the
/// pathfinding/best-Z call sites only ever read the disk cache.
/// Returns (nav_available, geo_available) after the attempt.
pub async fn ensure_zone(app_data: &Path, zone: &str) -> (bool, bool) {
    let dir = cache_dir(app_data);
    let _ = std::fs::create_dir_all(&dir);
    let mut ok = (false, false);
    for (sub, ext, slot) in [("nav", "nav", 0usize), ("base", "map", 1usize)] {
        let dest = dir.join(format!("{zone}.{ext}"));
        if dest.exists() {
            if slot == 0 {
                ok.0 = true;
            } else {
                ok.1 = true;
            }
            continue;
        }
        let url = format!("{RAW_BASE}/{sub}/{zone}.{ext}");
        let Ok(resp) = reqwest::get(&url).await else {
            continue;
        };
        if !resp.status().is_success() {
            continue;
        }
        let Ok(bytes) = resp.bytes().await else {
            continue;
        };
        // why: parse before persisting -- a 404 HTML page or truncated
        // body must not poison the cache
        let valid = if slot == 0 {
            parse_nav(&bytes).is_some()
        } else {
            parse_map(&bytes).is_some()
        };
        if valid && crate::diskwrite::write_atomic(&dest, &bytes).is_ok() {
            // why: drop the cached-negative so the next load re-reads
            if slot == 0 {
                nav_cache().lock_recover().remove(zone);
                ok.0 = true;
            } else {
                geo_cache().lock_recover().remove(zone);
                ok.1 = true;
            }
        }
    }
    ok
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Vec<u8> {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/emumaps");
        std::fs::read(p.join(name)).expect("fixture present")
    }

    #[test]
    fn blackburrow_nav_parses_into_a_connected_graph() {
        let nav = parse_nav(&fixture("blackburrow.nav")).expect("parses");
        assert!(nav.polys.len() > 2_000, "got {}", nav.polys.len());
        let with_edges = nav.polys.iter().filter(|p| !p.edges.is_empty()).count();
        assert!(
            with_edges * 10 > nav.polys.len() * 9,
            "adjacency rebuild connected {}/{} polys",
            with_edges,
            nav.polys.len()
        );
    }

    #[test]
    fn blackburrow_path_exists_and_stays_in_bounds() {
        let nav = parse_nav(&fixture("blackburrow.nav")).expect("parses");
        // two far-apart points inside the zone, MAP-FILE coords
        // (X[-489,397] Y[-349,254])
        let from = [-50.0, -30.0, 0.0];
        let to = [300.0, 100.0, -50.0];
        let path = nav.find_path(from, to).expect("route exists");
        assert!(path.len() >= 2);
        for p in &path {
            assert!(
                (-500.0..=400.0).contains(&p[0]) && (-350.0..=260.0).contains(&p[1]),
                "point off the map: {p:?}"
            );
        }
        // why: funnel output must not wildly exceed straight-line * a
        // sane detour factor for an open-ish route
        let straight = dist2(from, to).sqrt();
        let total: f32 = path.windows(2).map(|w| dist2(w[0], w[1]).sqrt()).sum();
        assert!(
            total < straight * 6.0,
            "path {total} vs straight {straight}"
        );
    }

    #[test]
    fn blackburrow_map_parses_and_best_z_answers_inside_the_zone() {
        let geo = parse_map(&fixture("blackburrow.map")).expect("parses");
        assert!(geo.tris.len() > 10_000);
        // a point well inside the zone footprint gets SOME ground; a
        // point far outside gets none
        let z = geo.best_z(0.0, 0.0, 0.0);
        assert!(z.is_some(), "no ground at zone center");
        assert!(geo.best_z(5_000.0, 5_000.0, 0.0).is_none());
    }

    /// why: the transform contract, pinned -- game -> nav -> game is identity
    #[test]
    fn coordinate_transforms_round_trip() {
        let p = [123.5, -42.25, 7.0];
        assert_eq!(nav_to_game(game_to_nav(p)), p);
    }
}

#[cfg(test)]
mod link_tests {
    use super::*;

    fn fixture(name: &str) -> Vec<u8> {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/emumaps");
        std::fs::read(p.join(name)).expect("fixture present")
    }

    /// why: the whole reason zone links exist -- Plane of Sky's mesh is
    /// 45 disconnected islands (measured), so island 1 to island 3 has
    /// no path without the portal edges, and a real path with them
    #[test]
    fn sky_islands_route_only_through_portal_links() {
        let mut nav = parse_nav(&fixture("airplane.nav")).expect("parses");
        // island 1 area -> island 3 (MAP-FILE coords from the map's own
        // portal labels, nudged off the pads themselves)
        let from = [-800.0, -1400.0, -671.0];
        let to = [-200.0, -180.0, -120.0];
        assert!(
            nav.find_path(from, to).is_none(),
            "no path without links -- disconnected islands"
        );
        nav.apply_links(zone_links("airplane"));
        let path = nav.find_path(from, to).expect("portal-linked path exists");
        assert!(path.len() >= 3, "got {} waypoints", path.len());
    }

    /// why: one-way stays one-way -- island 2 back to island 1 has no
    /// reverse edge (progression ports don't run backwards)
    #[test]
    fn portal_links_are_directed() {
        let mut nav = parse_nav(&fixture("airplane.nav")).expect("parses");
        nav.apply_links(zone_links("airplane"));
        let island2 = [580.0, 460.0, -364.0];
        let island1 = [-800.0, -1400.0, -671.0];
        // forward ring reaches island 8's return pad eventually, but the
        // DIRECT reverse hop 2 -> 1 must not exist as an edge; a route
        // may still exist the long way around the ring, which is
        // correct -- so assert on the immediate-edge level instead
        let p2 = nav.nearest_poly(island2).unwrap();
        let p1 = nav.nearest_poly(island1).unwrap();
        assert!(
            !nav.polys[p2 as usize].edges.iter().any(|e| e.to == p1),
            "no direct reverse edge island2 -> island1"
        );
    }
}

#[cfg(test)]
mod ground_hug_tests {
    use super::*;

    /// why: the reported bug's shape -- an open-terrain route straight in
    /// XY came back as two points and the drawn line cut through hills.
    /// With geo, a long leg subdivides and every sample sits on a real
    /// surface (best_z answers at that column).
    #[test]
    fn a_long_walk_leg_hugs_the_ground_instead_of_two_corners() {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/emumaps");
        let nav = parse_nav(&std::fs::read(p.join("blackburrow.nav")).unwrap()).unwrap();
        let geo = parse_map(&std::fs::read(p.join("blackburrow.map")).unwrap()).unwrap();
        let from = [-50.0, -30.0, 0.0];
        let to = [300.0, 100.0, -50.0];
        let legs = nav.find_route(from, to, Some(&geo)).expect("route");
        let n: usize = legs
            .iter()
            .map(|l| match l {
                NavLeg::Walk(w) => w.len(),
                _ => 0,
            })
            .sum();
        assert!(n > 10, "expected a sampled route, got {n} points");
        for l in &legs {
            if let NavLeg::Walk(w) = l {
                for pt in w {
                    if let Some(gz) = geo.best_z(pt[0], pt[1], pt[2]) {
                        assert!(
                            (gz - pt[2]).abs() < 1.0,
                            "sample floats off its own surface: {pt:?} vs ground {gz}"
                        );
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod funnel_tests {
    use super::*;

    /// why: the decisive hand-checkable case -- an L corridor. Straight
    /// from->to leaves the corridor, so the funnel MUST emit the inner
    /// corner. This is what the original implementation never did (zero
    /// corners, straight lines through walls, reported live in Feerrott).
    #[test]
    fn an_l_corridor_emits_the_inner_corner() {
        // cells: A(0,0) -> B(10,0) -> C(10,10); portal A->B at x=5,
        // y in [-2,2]; portal B->C at y=5, x in [8,12]
        let p1 = orient_portal(
            [5.0, -2.0, 0.0],
            [5.0, 2.0, 0.0],
            [0.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
        );
        let p2 = orient_portal(
            [8.0, 5.0, 0.0],
            [12.0, 5.0, 0.0],
            [10.0, 0.0, 0.0],
            [10.0, 10.0, 0.0],
        );
        let path = funnel([0.0, 0.0, 0.0], [10.0, 10.0, 0.0], &[p1, p2]);
        assert!(
            path.iter()
                .any(|p| (p[0] - 5.0).abs() < 0.01 && (p[1] - 2.0).abs() < 0.01),
            "must corner at (5,2), got {path:?}"
        );
        // and the reverse corridor bends at its own inner corner too
        let q1 = orient_portal(
            [8.0, 5.0, 0.0],
            [12.0, 5.0, 0.0],
            [10.0, 10.0, 0.0],
            [10.0, 0.0, 0.0],
        );
        let q2 = orient_portal(
            [5.0, -2.0, 0.0],
            [5.0, 2.0, 0.0],
            [10.0, 0.0, 0.0],
            [0.0, 0.0, 0.0],
        );
        let back = funnel([10.0, 10.0, 0.0], [0.0, 0.0, 0.0], &[q1, q2]);
        assert!(
            back.iter()
                .any(|p| (p[0] - 5.0).abs() < 0.01 && (p[1] - 2.0).abs() < 0.01),
            "reverse must corner at (5,2), got {back:?}"
        );
    }
}
