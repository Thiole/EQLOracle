//! why: dump a zone's navmesh poly centers (map coords) for outside analysis
//! run: cargo run --release -p eqlp-app --example dump_poly_centers -- <emu_maps dir> <zone> <out>
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let nav =
        eqlp_app::emumaps::parse_nav(&std::fs::read(format!("{}/{}.nav", a[0], a[1])).unwrap())
            .unwrap();
    let mut out = String::new();
    for p in &nav.polys {
        out.push_str(&format!(
            "{} {} {}\n",
            p.center[0], p.center[1], p.center[2]
        ));
    }
    std::fs::write(&a[2], out).unwrap();
    println!("{} centers", nav.polys.len());
}
