//! why: "when routing out of it it sometimes uses zones in the path that
//!      are out of era" -- routes out of a zone with and without the era
//!      ceiling and prints every hop plus that hop own era. Set
//!      EQLP_NO_PORTS=1 to force walking.
//! input: <install dir> [destinations...]
//! run: cargo run -p eqlp-app --release --example guk_route -- <dir> [zones]
use eqlp_app::gearplanner::{era_ix, CURRENT_ERA};
use eqlp_app::{routing, zonedata};
use std::collections::HashMap;
use std::path::Path;

fn era_of(zone: &str) -> String {
    zonedata::zones()
        .iter()
        .find(|z| z.name == zone)
        .and_then(|z| z.era.clone())
        .unwrap_or_else(|| "(none)".to_string())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let base = Path::new(&args[0]);
    let from = "Upper Guk";
    let dests: Vec<String> = if args.len() > 1 {
        args[1..].to_vec()
    } else {
        vec![
            "Freeport".into(),
            "Qeynos".into(),
            "Butcherblock Mountains".into(),
            "Greater Faydark".into(),
            "Plane of Sky".into(),
        ]
    };
    // why: a fully levelled caster, so the ports are never the variable
    // why: EQLP_NO_PORTS=1 -- a melee has to WALK out, which is the only
    // way a waypoint zone ever enters the path at all
    let levels: HashMap<String, u8> = if std::env::var("EQLP_NO_PORTS").is_ok() {
        HashMap::new()
    } else {
        ["Wizard", "Druid", "Enchanter"]
            .iter()
            .map(|c| (c.to_string(), 50u8))
            .collect()
    };
    let sky = era_ix(CURRENT_ERA);
    for to in &dests {
        for (label, ceiling) in [("no era ceiling", None), ("Sky Era ceiling", sky)] {
            let r = routing::find_zone_route_known(base, from, to, &levels, ceiling, None, None);
            match r {
                None => println!("{from} -> {to} [{label}]: NO ROUTE"),
                Some(route) => {
                    let bad: Vec<String> = route
                        .hops
                        .iter()
                        .map(|h| h.zone.clone())
                        .filter(|z| {
                            era_ix(&era_of(z)).is_some_and(|ix| sky.is_some_and(|s| ix > s))
                        })
                        .collect();
                    println!(
                        "{from} -> {to} [{label}]: {} hops{}",
                        route.hops.len(),
                        if bad.is_empty() {
                            String::new()
                        } else {
                            format!("  OUT OF ERA IN PATH: {bad:?}")
                        }
                    );
                    for h in &route.hops {
                        println!("     {:?} -> {:<30} ({})", h.kind, h.zone, era_of(&h.zone));
                    }
                }
            }
        }
        println!();
    }
}
