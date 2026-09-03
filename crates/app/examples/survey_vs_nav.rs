//! why: do surveyed spawn points sit ON the navmesh? Per point (map
//!      coords = (-locY, -locX, z)): nearest poly center distance and the
//!      z gap -- "units floating far above the castle" is either a bad
//!      transform (XY off) or a mesh that never covered that floor.
//! input: <emu_maps dir> <zone stem> <raw log zone name>
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let nav =
        eqlp_app::emumaps::parse_nav(&std::fs::read(format!("{}/{}.nav", a[0], a[1])).unwrap())
            .unwrap();
    let polys: Vec<[f32; 3]> = nav
        .polys
        .iter()
        .filter(|p| p.verts.len() > 2)
        .map(|p| p.center)
        .collect();
    let mut far = 0;
    let mut n = 0;
    let mut worst: Vec<(f32, f32, String)> = Vec::new();
    for s in eqlp_app::spawndata::spawns()
        .iter()
        .filter(|s| s.zone == a[2])
    {
        let p = [-s.y, -s.x, s.z];
        let (mut best_xy, mut best_dz) = (f32::MAX, 0.0f32);
        for c in &polys {
            let dxy = ((c[0] - p[0]).powi(2) + (c[1] - p[1]).powi(2)).sqrt();
            if dxy < best_xy {
                best_xy = dxy;
                best_dz = p[2] - c[2];
            }
        }
        n += 1;
        if best_xy > 12.0 || best_dz.abs() > 12.0 {
            far += 1;
        }
        worst.push((best_xy, best_dz, s.name.clone()));
    }
    worst.sort_by(|a, b| (b.0 + b.1.abs()).partial_cmp(&(a.0 + a.1.abs())).unwrap());
    println!("{n} survey points; {far} more than 12u (xy) or 12u (z) from the nearest poly center");
    for (dxy, dz, name) in worst.iter().take(8) {
        println!("  {name:<28} nearest poly {dxy:6.1}u away in xy, point is {dz:+6.1} in z");
    }
    let med = {
        let mut v: Vec<f32> = worst.iter().map(|w| w.1).collect();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v[v.len() / 2]
    };
    println!("median z gap (point - poly): {med:+.1}");
}
