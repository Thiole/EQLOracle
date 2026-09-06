//! why: "try random routes to many different zones. then verify their
//!      full route. verify every step is zones only in current era" --
//!      samples zone pairs, routes them under the era ceiling, and fails
//!      loudly on any hop landing in a zone the server does not have.
//!      Also reports pairs the ceiling made unroutable, so a filter that
//!      severs the map cannot pass quietly.
//! input: <install dir> [pairs] [--ports]
//! run: cargo run -p eqlp-app --release --example route_era_sweep -- <dir> 80
use eqlp_app::gearplanner::{era_ix, CURRENT_ERA};
use eqlp_app::{routing, zonedata};
use std::collections::HashMap;
use std::path::Path;

/// why: deterministic sample -- a failing sweep has to be re-runnable
fn lcg(state: &mut u64) -> u64 {
    *state = state
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    *state >> 33
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let base = Path::new(&args[0]);
    let pairs: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(80);
    let with_ports = args.iter().any(|a| a == "--ports");
    let sky = era_ix(CURRENT_ERA).expect("current era is in ERA_ORDER");

    let era_of = |zone: &str| -> Option<usize> {
        zonedata::zones()
            .iter()
            .find(|z| z.name == zone)
            .and_then(|z| z.era.as_deref())
            .and_then(era_ix)
    };
    let in_era = |zone: &str| era_of(zone).is_none_or(|ix| ix <= sky);

    let live: Vec<&str> = zonedata::zones()
        .iter()
        .map(|z| z.name.as_str())
        .filter(|n| in_era(n))
        .collect();
    println!(
        "{} zones at or before {CURRENT_ERA}, of {} in the pack",
        live.len(),
        zonedata::zones().len()
    );

    let levels: HashMap<String, u8> = if with_ports {
        ["Wizard", "Druid"]
            .iter()
            .map(|c| (c.to_string(), 50u8))
            .collect()
    } else {
        HashMap::new()
    };
    println!(
        "routing {pairs} random pairs {}\n",
        if with_ports {
            "WITH ports"
        } else {
            "on foot (no ports)"
        }
    );

    let mut seed = 0x5eed_1234_u64;
    let (mut routed, mut no_route, mut severed, mut violations) = (0, 0, 0, 0);
    for i in 0..pairs {
        let from = live[(lcg(&mut seed) as usize) % live.len()];
        let to = live[(lcg(&mut seed) as usize) % live.len()];
        if from == to {
            continue;
        }
        let capped = routing::find_zone_route_known(base, from, to, &levels, Some(sky), None, None);
        match &capped {
            None => {
                no_route += 1;
                // why: unroutable is only a FINDING if the ceiling caused
                // it -- plenty of pairs have no path at all
                if routing::find_zone_route_known(base, from, to, &levels, None, None, None)
                    .is_some()
                {
                    severed += 1;
                    println!("  SEVERED BY THE CEILING  {from} -> {to}");
                } else {
                    println!("  no path either way      {from} -> {to}");
                }
            }
            Some(r) => {
                routed += 1;
                for h in &r.hops {
                    if !in_era(&h.zone) {
                        violations += 1;
                        println!(
                            "  OUT OF ERA HOP  {from} -> {to}  via {} ({:?})",
                            h.zone,
                            era_of(&h.zone)
                        );
                    }
                }
            }
        }
        if i % 20 == 19 {
            println!("  ... {} pairs done", i + 1);
        }
    }
    println!(
        "\nrouted {routed}, no route {no_route} (of those, {severed} severed by the ceiling), out-of-era hops {violations}"
    );
    assert_eq!(violations, 0, "every hop must be a zone the server has");
}
