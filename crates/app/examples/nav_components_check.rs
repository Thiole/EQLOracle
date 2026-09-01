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
}
