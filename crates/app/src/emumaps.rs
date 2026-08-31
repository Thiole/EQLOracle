//! EQEmu navigation data: navmesh pathfinding and best-Z ground snapping.
//!
//! Sources (https://github.com/EQEmu/maps): `nav/<zone>.nav` -- a
//! Recast/Detour tiled navmesh in an `EQNAVMESH` zlib wrapper, the
//! walkable-surface graph the emulator's own NPCs path on;
//! `base/<zone>.map` -- the zone's collision triangle mesh (`best_z`).
//! The game's own `maps/<zone>.txt` line files stay the VIEW; these two
//! feed pathfinding and ground snapping only.
//!
//! Coordinates, established empirically against blackburrow
//! (docs/architecture.md): game(x,y,z) -> emu(-x,-y,z); nav = emu with
//! y/z swapped (Detour is Y-up). All public APIs here take and return
//! GAME coordinates.
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

use std::collections::{BinaryHeap, HashMap};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use crate::state::LockRecover;

// ---------------------------------------------------------------- parsing

#[derive(Debug)]
pub struct NavPoly {
    /// why: game-coordinate vertices, fan order
    pub verts: Vec<[f32; 3]>,
    /// why: (neighbor poly index, shared-edge a, shared-edge b) -- edge
    /// endpoints in game coords, the funnel's portal
    pub edges: Vec<(u32, [f32; 3], [f32; 3])>,
    pub center: [f32; 3],
}

#[derive(Debug)]
pub struct ZoneNav {
    pub polys: Vec<NavPoly>,
}

#[derive(Debug)]
pub struct ZoneGeo {
    verts: Vec<[f32; 3]>,
    tris: Vec<[u32; 3]>,
    /// why: XY-cell -> triangle indices, CELL-sized buckets for best_z
    grid: HashMap<(i32, i32), Vec<u32>>,
}

const CELL: f32 = 32.0;

pub fn nav_to_game(p: [f32; 3]) -> [f32; 3] {
    // nav(x, y_up, z) -> emu(x, z, y_up) -> game(-x, -z, y_up)
    [-p[0], -p[2], p[1]]
}

pub fn game_to_nav(p: [f32; 3]) -> [f32; 3] {
    [-p[0], p[2], -p[1]]
}

fn emu_to_game(p: [f32; 3]) -> [f32; 3] {
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

/// why: quantized endpoint pair as an undirected edge key -- float verts
/// shared across tiles are bit-identical in practice, but quantizing to
/// centimeters keeps the match robust to any serialization wobble
fn edge_key(a: [f32; 3], b: [f32; 3]) -> (i64, i64) {
    let q = |p: [f32; 3]| -> i64 {
        let x = (p[0] * 100.0).round() as i64;
        let y = (p[1] * 100.0).round() as i64;
        let z = (p[2] * 100.0).round() as i64;
        (x << 42) ^ (y << 21) ^ z
    };
    let (ka, kb) = (q(a), q(b));
    if ka <= kb {
        (ka, kb)
    } else {
        (kb, ka)
    }
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
    // (poly index, edge endpoints) per open (unmatched) boundary edge
    type OpenEdge = (u32, [f32; 3], [f32; 3]);
    let mut open_edges: HashMap<(i64, i64), OpenEdge> = HashMap::new();
    // off-mesh endpoints to connect after all polys exist
    let mut offmesh: Vec<([f32; 3], [f32; 3])> = Vec::new();

    for _ in 0..ntiles {
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
            let idx = polys.len() as u32;
            polys.push(NavPoly {
                edges: Vec::new(),
                center,
                verts: pverts,
            });
            // register/match undirected edges
            let pv = &polys[idx as usize].verts.clone();
            for k in 0..pv.len() {
                let a = pv[k];
                let b = pv[(k + 1) % pv.len()];
                let key = edge_key(a, b);
                if let Some((other, oa, ob)) = open_edges.remove(&key) {
                    polys[idx as usize].edges.push((other, oa, ob));
                    polys[other as usize].edges.push((idx, oa, ob));
                } else {
                    open_edges.insert(key, (idx, a, b));
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
                polys[pa as usize].edges.push((pb, a, b));
                polys[pb as usize].edges.push((pa, b, a));
            }
        }
    }

    (!polys.is_empty()).then_some(ZoneNav { polys })
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
}

// ---------------------------------------------------------------- pathfinding

impl ZoneNav {
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

    /// A* over poly adjacency + funnel smoothing. Game coords in and out.
    /// None when either endpoint has no nearby poly or no route exists.
    pub fn find_path(&self, from: [f32; 3], to: [f32; 3]) -> Option<Vec<[f32; 3]>> {
        let start = self.nearest_poly(from)?;
        let goal = self.nearest_poly(to)?;
        if start == goal {
            return Some(vec![from, to]);
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

        let h = |i: u32| dist2(self.polys[i as usize].center, to).sqrt();
        let mut g: HashMap<u32, f32> = HashMap::from([(start, 0.0)]);
        let mut came: HashMap<u32, (u32, [f32; 3], [f32; 3])> = HashMap::new();
        let mut heap = BinaryHeap::from([Node(h(start), start)]);
        while let Some(Node(_, cur)) = heap.pop() {
            if cur == goal {
                break;
            }
            let gc = g[&cur];
            for &(nb, ea, eb) in &self.polys[cur as usize].edges {
                let step = dist2(
                    self.polys[cur as usize].center,
                    self.polys[nb as usize].center,
                )
                .sqrt();
                let ng = gc + step;
                if g.get(&nb).is_none_or(|&old| ng < old) {
                    g.insert(nb, ng);
                    came.insert(nb, (cur, ea, eb));
                    heap.push(Node(ng + h(nb), nb));
                }
            }
        }
        came.contains_key(&goal).then_some(())?;

        // portal chain goal -> start, reversed
        let mut portals: Vec<([f32; 3], [f32; 3])> = Vec::new();
        let mut cur = goal;
        while cur != start {
            let (prev, ea, eb) = came[&cur];
            portals.push((ea, eb));
            cur = prev;
        }
        portals.reverse();
        Some(funnel(from, to, &portals))
    }
}

fn weighted(a: [f32; 3], b: [f32; 3]) -> f32 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    let dz = (a[2] - b[2]) * 4.0;
    dx * dx + dy * dy + dz * dz
}

/// Simple stupid funnel over the XY projection; Z rides along from
/// whichever portal endpoint each corner comes from.
fn funnel(from: [f32; 3], to: [f32; 3], portals: &[([f32; 3], [f32; 3])]) -> Vec<[f32; 3]> {
    let cross = |o: [f32; 3], a: [f32; 3], b: [f32; 3]| -> f32 {
        (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
    };
    // portals + a zero-width goal portal
    let mut ps: Vec<([f32; 3], [f32; 3])> = Vec::with_capacity(portals.len() + 1);
    ps.extend_from_slice(portals);
    ps.push((to, to));

    let mut path = vec![from];
    let mut apex = from;
    let (mut left, mut right) = (ps[0].0, ps[0].1);
    // orient the first portal so `left` really is left of travel
    if cross(apex, left, right) < 0.0 {
        std::mem::swap(&mut left, &mut right);
    }
    let (mut li, mut ri) = (0usize, 0usize);
    let mut i = 1;
    while i < ps.len() {
        let (mut a, mut b) = ps[i];
        if cross(apex, a, b) < 0.0 {
            std::mem::swap(&mut a, &mut b);
        }
        // tighten right
        if cross(apex, right, b) <= 0.0 {
            if apex == right || cross(apex, left, b) > 0.0 {
                right = b;
                ri = i;
            } else {
                path.push(left);
                apex = left;
                let restart = li + 1;
                left = apex;
                right = apex;
                li = restart - 1;
                ri = restart - 1;
                i = restart;
                continue;
            }
        }
        // tighten left
        if cross(apex, left, a) >= 0.0 {
            if apex == left || cross(apex, right, a) < 0.0 {
                left = a;
                li = i;
            } else {
                path.push(right);
                apex = right;
                let restart = ri + 1;
                left = apex;
                right = apex;
                li = restart - 1;
                ri = restart - 1;
                i = restart;
                continue;
            }
        }
        i += 1;
    }
    path.push(to);
    path
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
        .map(Arc::new);
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
        // two far-apart points inside the zone (game coords, from the
        // real map bounds X[-489,397] Y[-349,254])
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
