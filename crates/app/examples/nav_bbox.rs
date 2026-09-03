//! why: where the navmesh sits vs the drawn map -- bounding boxes and
//!      the share of polys inside the map's own extent
//! input: <emu_maps dir> <zone> <game maps dir>
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let nav =
        eqlp_app::emumaps::parse_nav(&std::fs::read(format!("{}/{}.nav", a[0], a[1])).unwrap())
            .unwrap();
    let mut lo = [f32::MAX; 3];
    let mut hi = [f32::MIN; 3];
    let mut zs: Vec<f32> = Vec::new();
    for p in nav.polys.iter().filter(|p| p.verts.len() > 2) {
        for k in 0..3 {
            lo[k] = lo[k].min(p.center[k]);
            hi[k] = hi[k].max(p.center[k]);
        }
        zs.push(p.center[2]);
    }
    zs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!(
        "nav polys {} bbox x[{:.0},{:.0}] y[{:.0},{:.0}] z[{:.0},{:.0}] median z {:.0}",
        zs.len(),
        lo[0],
        hi[0],
        lo[1],
        hi[1],
        lo[2],
        hi[2],
        zs[zs.len() / 2]
    );
    let txt = std::fs::read_to_string(format!("{}/{}.txt", a[2], a[1])).unwrap();
    let mut mlo = [f32::MAX; 3];
    let mut mhi = [f32::MIN; 3];
    for line in txt.lines().filter(|l| l.starts_with('L')) {
        let n: Vec<f32> = line[1..]
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();
        if n.len() >= 6 {
            for (k, v) in [
                (0, n[0]),
                (1, n[1]),
                (2, n[2]),
                (0, n[3]),
                (1, n[4]),
                (2, n[5]),
            ] {
                mlo[k] = mlo[k].min(v);
                mhi[k] = mhi[k].max(v);
            }
        }
    }
    println!(
        "map  bbox x[{:.0},{:.0}] y[{:.0},{:.0}] z[{:.0},{:.0}]",
        mlo[0], mhi[0], mlo[1], mhi[1], mlo[2], mhi[2]
    );
}
