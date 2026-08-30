//! why: empirical pathfind probe on a real map file -- routes between
//! its own markers and brute-force counts wall-crossing legs (the
//! "jumps lines" bug class); prints timing so the along-the-leg
//! blocked() cost stays visible.
//! run: cargo run -p eqlp-app --release --example pathfind_check -- <map.txt>
use eqlp_app::mapsdata::parse_map_text;
use eqlp_app::pathfind::find_path;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: pathfind_check <map.txt>");
    // why: markers live in the _1/_2/_3 layer files, lines in the base --
    // merge all layers the way the app's own loader does
    let mut text = std::fs::read_to_string(&path).expect("read map");
    let stem = path.trim_end_matches(".txt");
    for n in 1..=3 {
        if let Ok(layer) = std::fs::read_to_string(format!("{stem}_{n}.txt")) {
            text.push('\n');
            text.push_str(&layer);
        }
    }
    let map = parse_map_text(&text);
    println!(
        "{}: {} lines, {} markers",
        path,
        map.lines.len(),
        map.markers.len()
    );

    let mut routes = 0;
    let mut none = 0;
    let mut crossings = 0;
    let mut endpoint_crossings = 0;
    let t0 = std::time::Instant::now(); // clock-exempt: perf probe measuring real wall time
    let markers = &map.markers;
    for i in 0..markers.len() {
        for j in (i + 1)..markers.len() {
            let a = markers[i].pos;
            let b = markers[j].pos;
            let Some(p) = find_path(&map, (a.x, a.y, a.z), (b.x, b.y, b.z)) else {
                none += 1;
                continue;
            };
            routes += 1;
            for (wi, w) in p.windows(2).enumerate() {
                let endpoint_leg = wi == 0 || wi == p.len() - 2;
                for l in &map.lines {
                    // why: same grayscale wall filter the grid applies
                    if !(l.r == l.g && l.g == l.b_) {
                        continue;
                    }
                    let (z_lo, z_hi) = (a.z - 40.0, a.z + 40.0);
                    let (slo, shi) = (l.a.z.min(l.b.z), l.a.z.max(l.b.z));
                    if shi < z_lo || slo > z_hi {
                        continue;
                    }
                    if eqlp_app::pathfind::segments_intersect_for_test(
                        w[0].0, w[0].1, w[1].0, w[1].1, l.a.x, l.a.y, l.b.x, l.b.y,
                    ) {
                        if endpoint_leg {
                            endpoint_crossings += 1;
                        } else {
                            crossings += 1;
                        }
                    }
                }
            }
        }
    }
    println!(
        "{} routes found, {} unreachable, {} mid-path crossings, {} endpoint-snap crossings, {:?} total",
        routes,
        none,
        crossings,
        endpoint_crossings,
        t0.elapsed()
    );
}
