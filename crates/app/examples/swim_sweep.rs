//! why: "every mob in the zone gets a valid route" -- route from one start
//! to every wiki spawn point of a zone; per NPC: reachable, waypoints,
//! outside-the-volume waypoints (dry AND not enclosed), strict 3D hits.
//! run: cargo run --release -p eqlp-app --example swim_sweep -- <emu_maps dir> <game maps dir> <zone key> "<Zone Name>" fx fy fz
use eqlp_app::emumaps::{self, NavLeg};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (cache, maps, zone, zone_name) = (&a[0], &a[1], &a[2], &a[3]);
    let f: Vec<f32> = a[4..7].iter().map(|s| s.trim().parse().unwrap()).collect();
    let from = [f[0], f[1], f[2]];
    let geo = emumaps::parse_map(&std::fs::read(format!("{cache}/{zone}.map")).unwrap()).unwrap();
    let mut nav =
        emumaps::parse_nav(&std::fs::read(format!("{cache}/{zone}.nav")).unwrap()).unwrap();
    nav.apply_links(emumaps::zone_links(zone));
    let base = std::path::Path::new(maps).parent().unwrap();
    let walls = eqlp_app::mapsdata::load_zone_map(
        base,
        std::env::var("EQLP_MAP_PACK").ok().as_deref(),
        zone,
    )
    .ok()
    .map(|m| emumaps::WallSet::from_lines(&m.lines));
    let water = std::fs::read(format!("{cache}/{zone}.wtr"))
        .ok()
        .and_then(|b| emumaps::parse_water(&b))
        .map(std::sync::Arc::new);
    if emumaps::is_underwater(zone) {
        nav.swim = true;
        nav.walls = walls;
        nav.water = water.clone();
        nav.bridge_gaps(&geo);
    }
    let walls_chk = eqlp_app::mapsdata::load_zone_map(
        base,
        std::env::var("EQLP_MAP_PACK").ok().as_deref(),
        zone,
    )
    .ok()
    .map(|m| emumaps::WallSet::from_lines(&m.lines));
    // why: a mesh floor poly is inside by definition -- only swim-node
    // waypoints can be "outside"
    let mesh_pts: std::collections::HashSet<(i32, i32, i32)> = nav
        .polys
        .iter()
        .filter(|p| p.verts.len() > 1)
        .map(|p| {
            (
                (p.center[0] * 10.0) as i32,
                (p.center[1] * 10.0) as i32,
                ((p.center[2] + 2.5) * 10.0) as i32,
            )
        })
        .collect();
    let inside = |p: [f32; 3]| -> bool {
        mesh_pts.contains(&(
            (p[0] * 10.0) as i32,
            (p[1] * 10.0) as i32,
            (p[2] * 10.0) as i32,
        )) || water.as_ref().is_some_and(|w| w.is_water(p))
            || walls_chk.as_ref().is_some_and(|w| w.encloses(p, 30.0))
    };
    let mut ok = 0;
    let mut fail = 0;
    let mut leaky = 0;
    let markers = eqlp_app::npcdata::markers_for_zone(zone_name);
    for (name, x, y, z) in &markers {
        // /loc -> map-file, same transform the UI applies; unknown z is
        // resolved exactly as find_walk_path does
        let mut to = [-y, -x, z.unwrap_or(0.0)];
        if z.is_none() {
            if let Some(rz) =
                emumaps::resolve_unknown_z(&geo, water.as_deref(), to[0], to[1], from[2])
            {
                to[2] = rz;
            }
        }
        match nav.find_route(from, to, Some(&geo)) {
            None => {
                fail += 1;
                println!(
                    "FAIL  {name:32} to=({:.0},{:.0},{:.0})",
                    to[0], to[1], to[2]
                );
            }
            Some(route) => {
                let mut pts: Vec<[f32; 3]> = Vec::new();
                for leg in &route {
                    if let NavLeg::Walk(w) = leg {
                        pts.extend_from_slice(w);
                    }
                }
                // ignore the raw endpoints (the target's z is a wiki guess)
                let out = pts[1..pts.len().saturating_sub(1)]
                    .iter()
                    .filter(|&&p| !inside(p))
                    .count();
                let hits = pts
                    .windows(2)
                    .filter(|w| match &walls_chk {
                        Some(ws) => geo.segment_hits_non_door(w[0], w[1], ws),
                        None => geo.segment_hits_any_tri(w[0], w[1]),
                    })
                    .count();
                if out > 0 {
                    leaky += 1;
                }
                ok += 1;
                println!(
                    "{}  {name:32} wps={:<3} outside={out:<2} hits={hits}",
                    if out > 0 { "LEAK " } else { "ok   " },
                    pts.len()
                );
            }
        }
    }
    println!(
        "== {} NPC spawn points: {ok} routed ({leaky} with outside waypoints), {fail} unreachable",
        markers.len()
    );
}
