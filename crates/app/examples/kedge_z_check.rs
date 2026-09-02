//! why: how a z-less wiki spawn point resolves in a stacked swim zone --
//!      every floor surface under its XY, which is wet, which was picked
//! input: <emu_maps dir> <zone> <zone name>
use eqlp_app::emumaps;
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (cache, zone, zone_name) = (&a[0], &a[1], &a[2]);
    let geo = emumaps::parse_map(&std::fs::read(format!("{cache}/{zone}.map")).unwrap()).unwrap();
    let water =
        emumaps::parse_water(&std::fs::read(format!("{cache}/{zone}.wtr")).unwrap()).unwrap();
    let markers = eqlp_app::npcdata::markers_for_zone(zone_name);
    let (mut with_z, mut multi) = (0, 0);
    for (name, x, y, z) in &markers {
        let (mx, my) = (-y, -x);
        if z.is_some() {
            with_z += 1;
        }
        let mut surfaces: Vec<(f32, bool)> = Vec::new();
        let mut h = 400.0f32;
        while h > -400.0 {
            if let Some(fz) = geo.best_z(mx, my, h) {
                if !surfaces.iter().any(|&(s, _)| (s - fz).abs() < 1.0) {
                    surfaces.push((fz, water.is_water([mx, my, fz + 3.0])));
                }
            }
            h -= 10.0;
        }
        let wet: Vec<f32> = surfaces.iter().filter(|s| s.1).map(|s| s.0).collect();
        if wet.len() > 1 {
            multi += 1;
        }
        let picked = emumaps::resolve_unknown_z(&geo, Some(&water), mx, my, 299.0);
        println!(
            "{name:<28} z={:?} picked={:?} wet_floors={:?} all={:?}",
            z,
            picked,
            wet,
            surfaces.iter().map(|s| s.0 as i32).collect::<Vec<_>>()
        );
    }
    println!(
        "{} markers, {with_z} with z, {multi} with >1 wet floor",
        markers.len()
    );
}
