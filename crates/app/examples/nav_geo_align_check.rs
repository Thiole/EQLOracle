//! why: are the navmesh and collision mesh in the same space? For each
//! nav poly center, is there a collision surface within 6u of its z in
//! its XY -- under the identity transform and under sign/swap variants.
//! run: cargo run --release -p eqlp-app --example nav_geo_align_check -- <emu_maps dir> <zone>...
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let cache = &a[0];
    for zone in &a[1..] {
        let (Ok(nb), Ok(gb)) = (
            std::fs::read(format!("{cache}/{zone}.nav")),
            std::fs::read(format!("{cache}/{zone}.map")),
        ) else {
            continue;
        };
        let nav = eqlp_app::emumaps::parse_nav(&nb).expect("nav");
        let geo = eqlp_app::emumaps::parse_map(&gb).expect("geo");
        type Xf = fn([f32; 3]) -> [f32; 3];
        let variants: [(&str, Xf); 6] = [
            ("id   (x,y)", |p| [p[0], p[1], p[2]]),
            ("swap (y,x)", |p| [p[1], p[0], p[2]]),
            ("neg  (-x,-y)", |p| [-p[0], -p[1], p[2]]),
            ("(-x,y)", |p| [-p[0], p[1], p[2]]),
            ("(x,-y)", |p| [p[0], -p[1], p[2]]),
            ("(-y,-x)", |p| [-p[1], -p[0], p[2]]),
        ];
        let step = (nav.polys.len() / 400).max(1);
        let sample: Vec<[f32; 3]> = nav.polys.iter().step_by(step).map(|p| p.center).collect();
        print!("{zone}: {} sampled polys;", sample.len());
        for (name, f) in variants {
            let hit = sample
                .iter()
                .filter(|&&c| {
                    let q = f(c);
                    geo.best_z(q[0], q[1], q[2])
                        .is_some_and(|z| (z - q[2]).abs() <= 6.0)
                })
                .count();
            print!("  {name}: {:.0}%", 100.0 * hit as f32 / sample.len() as f32);
        }
        println!();
    }
}
