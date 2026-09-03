//! why: what the rotation sees -- cast, recast, reuse group, dps per spell,
//!      with the install's own timer file loaded
//! input: <install dir> [spell names...]
use eqlp_app::dpscalc::list_damage_spells;
use eqlp_app::ingest::Ingest;
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let ing = Ingest::default();
    let all = list_damage_spells(&ing, true, Some(std::path::Path::new(&a[0])));
    for s in all {
        if a.len() > 1 && !a[1..].iter().any(|n| n == &s.name) {
            continue;
        }
        println!(
            "{:28} cast={:>4} recast={:>5} group={:?} dot={} dps_reuse={:.0} dps_noreuse={:.0} total={:.0}",
            s.name, s.casting_time, s.recast_time, s.reuse_group, s.is_dot, s.dps_with_reuse, s.dps_ignoring_reuse, s.total_damage
        );
    }
}
