//! why: audit a cached EQEmu navmesh's connectivity -- poly/edge counts,
//! connected-component sizes, and which components two points bind to
//! (the "no walkable route" report shape in one look).
//! run: cargo run --release -p eqlp-app --example nav_components_check -- <emu_maps dir> <zone> [fx fy fz tx ty tz]

use std::collections::VecDeque;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (cache, zone) = (&a[0], &a[1]);
    let nav_bytes = std::fs::read(format!("{cache}/{zone}.nav")).expect("nav cached");
    let mut nav = eqlp_app::emumaps::parse_nav(&nav_bytes).expect("nav parses");
    // mirror load_nav: swim-bridge underwater zones when geo is cached
    if eqlp_app::emumaps::is_underwater(zone) {
        if let Ok(geo_bytes) = std::fs::read(format!("{cache}/{zone}.map")) {
            let geo = eqlp_app::emumaps::parse_map(&geo_bytes).expect("geo parses");
            nav.swim = true;
            // EQLP_MAPS_DIR=<install>/maps enables the drawn-wall barrier
            if let Ok(maps) = std::env::var("EQLP_MAPS_DIR") {
                let base = std::path::Path::new(&maps)
                    .parent()
                    .expect("maps dir has a parent");
                nav.walls = eqlp_app::mapsdata::load_zone_map(base, None, zone)
                    .ok()
                    .map(|m| eqlp_app::emumaps::WallSet::from_lines(&m.lines));
            }
            nav.bridge_gaps(&geo);
        }
    }

    let n = nav.polys.len();
    let edges: usize = nav.polys.iter().map(|p| p.edges.len()).sum();
    println!("{zone}: {n} polys, {edges} directed edges");

    // undirected components over poly adjacency
    let mut comp = vec![u32::MAX; n];
    let mut sizes: Vec<(u32, usize)> = Vec::new();
    let mut c = 0u32;
    for s in 0..n {
        if comp[s] != u32::MAX {
            continue;
        }
        let mut size = 0usize;
        let mut q = VecDeque::from([s]);
        comp[s] = c;
        while let Some(i) = q.pop_front() {
            size += 1;
            for e in &nav.polys[i].edges {
                let t = e.to as usize;
                if comp[t] == u32::MAX {
                    comp[t] = c;
                    q.push_back(t);
                }
            }
        }
        sizes.push((c, size));
        c += 1;
    }
    sizes.sort_by_key(|&(_, s)| std::cmp::Reverse(s));
    println!(
        "{} components; largest: {:?}",
        sizes.len(),
        &sizes[..sizes.len().min(8)]
    );

    if a.len() >= 8 {
        let f: Vec<f32> = a[2..5].iter().map(|s| s.trim().parse().unwrap()).collect();
        let t: Vec<f32> = a[5..8].iter().map(|s| s.trim().parse().unwrap()).collect();
        for (name, p) in [("from", [f[0], f[1], f[2]]), ("to", [t[0], t[1], t[2]])] {
            if let Some(pi) = nav.nearest_poly_pub(p) {
                let poly = &nav.polys[pi as usize];
                println!(
                    "{name}: {p:?} -> poly {pi} center={:?} edges={} component={} (size {})",
                    poly.center,
                    poly.edges.len(),
                    comp[pi as usize],
                    sizes
                        .iter()
                        .find(|&&(id, _)| id == comp[pi as usize])
                        .map(|&(_, s)| s)
                        .unwrap_or(0),
                );
            }
        }
    }

    if a.len() >= 8 {
        if let Ok(geo_bytes) = std::fs::read(format!("{cache}/{zone}.map")) {
            let geo = eqlp_app::emumaps::parse_map(&geo_bytes).expect("geo parses");
            let f: Vec<f32> = a[2..5].iter().map(|s| s.trim().parse().unwrap()).collect();
            if let Some(fi) = nav.nearest_poly_pub([f[0], f[1], f[2]]) {
                let fc = comp[fi as usize];
                let island: Vec<usize> = (0..n).filter(|&i| comp[i] == fc).collect();
                let bb = |ids: &[usize]| {
                    let mut lo = [f32::MAX; 3];
                    let mut hi = [f32::MIN; 3];
                    for &i in ids {
                        for k in 0..3 {
                            lo[k] = lo[k].min(nav.polys[i].center[k]);
                            hi[k] = hi[k].max(nav.polys[i].center[k]);
                        }
                    }
                    (lo, hi)
                };
                println!(
                    "start island: {} polys, bbox {:?}",
                    island.len(),
                    bb(&island)
                );
                println!("whole mesh bbox {:?}", bb(&(0..n).collect::<Vec<_>>()));
                let mut hits: Vec<(f32, usize, usize)> = Vec::new();
                for &i in &island {
                    #[allow(clippy::needless_range_loop)] // why: j indexes comp AND polys
                    for j in 0..n {
                        if comp[j] == fc {
                            continue;
                        }
                        let mut pa = nav.polys[i].center;
                        pa[2] += 2.0;
                        let mut pb = nav.polys[j].center;
                        pb[2] += 2.0;
                        if geo.los_clear(pa, pb) {
                            let d = ((pa[0] - pb[0]).powi(2)
                                + (pa[1] - pb[1]).powi(2)
                                + (pa[2] - pb[2]).powi(2))
                            .sqrt();
                            hits.push((d, i, j));
                        }
                    }
                }
                hits.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap());
                println!("LOS-clear pairs from island to other comps: {}", hits.len());
                let mut per: std::collections::BTreeMap<u32, (usize, f32)> =
                    std::collections::BTreeMap::new();
                for (d, _, j) in &hits {
                    let e = per.entry(comp[*j]).or_insert((0, f32::MAX));
                    e.0 += 1;
                    e.1 = e.1.min(*d);
                }
                for (c, (cnt, mind)) in per {
                    let size = sizes
                        .iter()
                        .find(|&&(id, _)| id == c)
                        .map(|&(_, s)| s)
                        .unwrap_or(0);
                    println!("  -> comp {c} (size {size}): {cnt} clear pairs, nearest {mind:.0}");
                }
            }
        }
    }
}
