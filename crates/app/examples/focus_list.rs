//! why: what the DPS model's focus reader sees on your worn gear and its sockets
//! input: <install dir>
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    for f in eqlp_app::focus::equipped(std::path::Path::new(&a[0])) {
        println!(
            "{:?} {}-{}% from {} via {} maxlvl={:?} detr={} benef={} mindur={:?} maxdur={:?} mincast={:?} ae_ex={} hp={} other={}",
            f.kind, f.lo, f.hi, f.name, f.item, f.max_level, f.detrimental_only, f.beneficial_only,
            f.min_duration_secs, f.max_duration_secs, f.min_casting_time, f.exclude_ae, f.current_hp_only, f.other_effect_only
        );
    }
}
