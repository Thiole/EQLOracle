//! why: replay one real navmesh route query against cached EQEmu files
//!      and audit it -- endpoint snap distances, leg shapes, and XY
//!      crossings against the game map's own wall segments (the exact
//!      "line jumps through geometry" report shape).
//! input: <emu_maps dir> <game maps dir> <zone> fx fy fz tx ty tz  (map-file coords)
//! run: cargo run --release -p eqlp-app --example nav_route_check -- ...

use eqlp_app::emumaps::{self, NavLeg};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (cache, maps, zone) = (&a[0], &a[1], &a[2]);
    // why: trim -- negatives arrive space-prefixed to dodge clap-style
    // flag parsing
    let f: Vec<f32> = a[3..6].iter().map(|s| s.trim().parse().unwrap()).collect();
    let t: Vec<f32> = a[6..9].iter().map(|s| s.trim().parse().unwrap()).collect();
    let from = [f[0], f[1], f[2]];
    let to = [t[0], t[1], t[2]];

    let nav_bytes = std::fs::read(format!("{cache}/{zone}.nav")).expect("nav cached");
    let geo_bytes = std::fs::read(format!("{cache}/{zone}.map")).expect("map cached");
    let geo = emumaps::parse_map(&geo_bytes).expect("geo parses");
    let nav = {
        let mut n = emumaps::parse_nav(&nav_bytes).expect("nav parses");
        n.apply_links(emumaps::zone_links(zone));
        // mirror load_nav: swim-bridge underwater zones
        if emumaps::is_underwater(zone) {
            n.swim = true;
            n.bridge_gaps(&geo);
        }
        n
    };

    // endpoint snap audit
    for (name, p) in [("from", from), ("to", to)] {
        let ground = geo.best_z(p[0], p[1], p[2]);
        println!("{name}: {p:?} ground_here={ground:?}");
    }

    let Some(route) = nav.find_route(from, to, Some(&geo)) else {
        println!("NO ROUTE");
        return;
    };
    let mut flat: Vec<[f32; 3]> = Vec::new();
    for leg in &route {
        match leg {
            NavLeg::Walk(w) => {
                println!(
                    "walk leg: {} pts, first={:?} last={:?}",
                    w.len(),
                    w[0],
                    w[w.len() - 1]
                );
                flat.extend_from_slice(w);
            }
            NavLeg::Hop { at, to, label } => println!("hop: {label} {at:?} -> {to:?}"),
        }
    }

    // wall-crossing audit vs the game map's own L segments (XY only,
    // ignoring z -- deliberately the same naive read a player's eye does
    // on the top-down view)
    let map_txt = std::fs::read_to_string(format!("{maps}/{zone}.txt")).expect("game map present");
    type Wall = ((f32, f32), (f32, f32), f32, f32);
    let mut walls: Vec<Wall> = Vec::new();
    for line in map_txt.lines() {
        let Some(rest) = line.strip_prefix('L') else {
            continue;
        };
        let nums: Vec<f32> = rest
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();
        if nums.len() >= 6 {
            walls.push(((nums[0], nums[1]), (nums[3], nums[4]), nums[2], nums[5]));
        }
    }
    let cross = |a: (f32, f32), b: (f32, f32), c: (f32, f32), d: (f32, f32)| -> bool {
        let ccw = |p: (f32, f32), q: (f32, f32), r: (f32, f32)| {
            (r.1 - p.1) * (q.0 - p.0) > (q.1 - p.1) * (r.0 - p.0)
        };
        ccw(a, c, d) != ccw(b, c, d) && ccw(a, b, c) != ccw(a, b, d)
    };
    let mut crossings = 0;
    let mut near_z_crossings = 0;
    for w in flat.windows(2) {
        let (p, q) = ((w[0][0], w[0][1]), (w[1][0], w[1][1]));
        for &(wa, wb, z1, z2) in &walls {
            if cross(p, q, wa, wb) {
                crossings += 1;
                let seg_z = (w[0][2] + w[1][2]) / 2.0;
                if (z1.min(z2) - 15.0..=z1.max(z2) + 15.0).contains(&seg_z) {
                    near_z_crossings += 1;
                }
            }
        }
    }
    // strict 3D audit: brute-force every collision triangle per segment
    let mut hits_3d = 0;
    for (i, w) in flat.windows(2).enumerate() {
        if geo.segment_hits_any_tri(w[0], w[1]) {
            hits_3d += 1;
            println!("  HIT seg {i}/{}: {:?} -> {:?}", flat.len() - 1, w[0], w[1]);
        }
    }
    println!(
        "3D collision audit: {hits_3d} of {} segments intersect the collision mesh",
        flat.len().saturating_sub(1)
    );
    let xs: Vec<f32> = flat.iter().map(|p| p[0]).collect();
    let ys: Vec<f32> = flat.iter().map(|p| p[1]).collect();
    let len: f32 = flat
        .windows(2)
        .map(|w| {
            let dx = w[1][0] - w[0][0];
            let dy = w[1][1] - w[0][1];
            (dx * dx + dy * dy).sqrt()
        })
        .sum();
    println!(
        "{} waypoints; path len {len:.0}; X[{:.0},{:.0}] Y[{:.0},{:.0}]; XY wall crossings: {crossings} (within +-15 z: {near_z_crossings})",
        flat.len(),
        xs.iter().cloned().fold(f32::MAX,f32::min), xs.iter().cloned().fold(f32::MIN,f32::max),
        ys.iter().cloned().fold(f32::MAX,f32::min), ys.iter().cloned().fold(f32::MIN,f32::max),
    );
}
