//! why: which marker a non-live Maps view starts a route from -- the
//!      viewer takes markers[0] of the merged pack
//! input: <install base dir> <pack or -> <zone>
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let pack = (a[1] != "-").then_some(a[1].as_str());
    let m = eqlp_app::mapsdata::load_zone_map(std::path::Path::new(&a[0]), pack, &a[2]).unwrap();
    for (i, mk) in m.markers.iter().take(4).enumerate() {
        println!(
            "{i}: {:?} at ({:.0}, {:.0}, {:.0})",
            mk.label, mk.pos.x, mk.pos.y, mk.pos.z
        );
    }
}
