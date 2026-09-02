//! why: for a candidate bridge a->b (map coords), say which swim check
//! blocks it: collision LOS, drawn wall crossing, water runs, distance
//! run: cargo run --release -p eqlp-app --example gap_check -- <emu_maps> <game maps> <zone> ax ay az bx by bz
use eqlp_app::emumaps;
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (cache, maps, zone) = (&a[0], &a[1], &a[2]);
    let v: Vec<f32> = a[3..9].iter().map(|s| s.trim().parse().unwrap()).collect();
    let la: f32 = std::env::var("EQLP_LIFT_A")
        .ok()
        .and_then(|x| x.parse().ok())
        .unwrap_or(2.5);
    let lb: f32 = std::env::var("EQLP_LIFT_B")
        .ok()
        .and_then(|x| x.parse().ok())
        .unwrap_or(2.5);
    let (p, q) = ([v[0], v[1], v[2] + la], [v[3], v[4], v[5] + lb]);
    let geo = emumaps::parse_map(&std::fs::read(format!("{cache}/{zone}.map")).unwrap()).unwrap();
    let base = std::path::Path::new(maps).parent().unwrap();
    let walls = eqlp_app::mapsdata::load_zone_map(base, None, zone)
        .ok()
        .map(|m| emumaps::WallSet::from_lines(&m.lines));
    let water = std::fs::read(format!("{cache}/{zone}.wtr"))
        .ok()
        .and_then(|b| emumaps::parse_water(&b));
    let d = ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2)).sqrt();
    println!("dist {d:.1}");
    println!("collision LOS clear: {}", geo.los_clear(p, q));
    println!(
        "collision LOS clear (brute): {}",
        !geo.segment_hits_any_tri(p, q)
    );
    if let Some(t) = geo.first_hit_tri(p, q) {
        println!("first hit tri: {:?}", t);
    }
    if let Some(w) = &walls {
        println!(
            "drawn wall crossed: {} lengths={:?}",
            w.crosses(p, q),
            w.crossed_lengths(p, q)
        );
        println!(
            "a enclosed: {}  b enclosed: {}",
            w.encloses(p, 30.0),
            w.encloses(q, 30.0)
        );
    }
    if let Some(wm) = &water {
        println!(
            "water: a={} b={} segment_ok={} all_water={}",
            wm.is_water(p),
            wm.is_water(q),
            wm.segment_ok(p, q),
            wm.all_water(p, q)
        );
    }
}
