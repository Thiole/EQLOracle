//! why: list every collision-mesh surface under an XY (z hints stepped)
//! run: cargo run --release -p eqlp-app --example geo_column_check -- <emu_maps dir> <zone> x y [x y ...]
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let geo_bytes = std::fs::read(format!("{}/{}.map", a[0], a[1])).expect("map cached");
    let geo = eqlp_app::emumaps::parse_map(&geo_bytes).expect("geo parses");
    let mut i = 2;
    while i + 1 < a.len() {
        let (x, y): (f32, f32) = (
            a[i].trim().parse().unwrap(),
            a[i + 1].trim().parse().unwrap(),
        );
        let mut zs: Vec<i32> = Vec::new();
        let mut hint = 400.0f32;
        while hint > -400.0 {
            if let Some(z) = geo.best_z(x, y, hint) {
                let zi = z.round() as i32;
                if !zs.contains(&zi) {
                    zs.push(zi);
                }
            }
            hint -= 10.0;
        }
        zs.sort_unstable_by(|p, q| q.cmp(p));
        println!("surfaces under ({x}, {y}): {zs:?}");
        i += 2;
    }
}
