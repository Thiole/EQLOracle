//! why: is the water volume rooms or a hull? Fraction of grid points that
//!      read wet, far from any navmesh poly vs near one.
//! input: <emu_maps dir> <zone>
use eqlp_app::emumaps;
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (cache, zone) = (&a[0], &a[1]);
    let water =
        emumaps::parse_water(&std::fs::read(format!("{cache}/{zone}.wtr")).unwrap()).unwrap();
    let nav = emumaps::parse_nav(&std::fs::read(format!("{cache}/{zone}.nav")).unwrap()).unwrap();
    let centers: Vec<[f32; 3]> = nav
        .polys
        .iter()
        .filter(|p| p.verts.len() > 1)
        .map(|p| p.center)
        .collect();
    let (mut near_wet, mut near_n, mut far_wet, mut far_n) = (0, 0, 0, 0);
    let mut z = -340.0f32;
    while z < 360.0 {
        let mut x = -270.0f32;
        while x < 550.0 {
            let mut y = -300.0f32;
            while y < 370.0 {
                let p = [x, y, z];
                let dmin = centers
                    .iter()
                    .map(|c| ((c[0] - x).powi(2) + (c[1] - y).powi(2) + (c[2] - z).powi(2)).sqrt())
                    .fold(f32::MAX, f32::min);
                let wet = water.is_water(p);
                if dmin < 20.0 {
                    near_n += 1;
                    near_wet += wet as u32;
                } else if dmin > 80.0 {
                    far_n += 1;
                    far_wet += wet as u32;
                }
                y += 20.0;
            }
            x += 20.0;
        }
        z += 20.0;
    }
    println!(
        "near mesh (<20u): {near_wet}/{near_n} wet; far from mesh (>80u): {far_wet}/{far_n} wet"
    );
}
