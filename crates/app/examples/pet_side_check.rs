//! why: audit "my pet shows up as incoming damage while attacking the
//!      enemy" against a real log. Finds entities the UI classifies
//!      ENEMY (allegiance) whose damage goes into OTHER enemies and who
//!      are never hit back by the player's side -- real mobs get
//!      attacked, your own pets don't.
//! input: path to a real log
//! output: suspect entities ranked by damage, with per-name evidence
//! run: cargo run --release -p eqlp-app --example pet_side_check -- <log>

use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;
use eqlp_store::EventKind;
use std::collections::HashMap;

#[derive(Default)]
struct Tally {
    dmg_into_enemies: u64,
    dmg_into_allies: u64,
    dmg_taken_from_allies: u64,
    hits_on_anchor: u64,
    fights: std::collections::HashSet<u32>,
}

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: pet_side_check <path-to-log>");
    let raw = std::fs::read(&path).unwrap_or_else(|e| panic!("couldn't read {path}: {e}"));
    let lines = framed_lines(&raw);
    let engine = build_engine().expect("pack builds");
    let mut ing = Ingest::default();
    for chunk in lines.chunks(100_000) {
        backfill_lines(&mut ing, &engine, chunk, 8);
    }

    // per-name evidence across every fight the player was in
    let mut tallies: HashMap<String, Tally> = HashMap::new();

    let encs: Vec<_> = ing
        .store
        .encounters
        .iter()
        .filter(|e| e.involves_you && e.end_ms.is_some())
        .cloned()
        .collect();
    for e in &encs {
        for i in e.range() {
            if ing.store.enc[i] != e.id.0 || ing.store.kind[i] != EventKind::Damage {
                continue;
            }
            let ts = ing.store.ts[i];
            let actor = ing.store.name(ing.store.actor[i]).to_string();
            let target = ing.store.name(ing.store.target[i]).to_string();
            let amt = ing.store.amount[i];
            let actor_enemy = ing.allegiance_at(&actor, ts).is_enemy();
            let target_enemy = ing.allegiance_at(&target, ts).is_enemy();
            if actor_enemy {
                let t = tallies.entry(actor.clone()).or_default();
                if target_enemy {
                    t.dmg_into_enemies += amt;
                } else {
                    t.dmg_into_allies += amt;
                }
                if ing.store.target[i] == e.target {
                    t.hits_on_anchor += amt;
                }
                t.fights.insert(e.id.0);
            }
            if target_enemy && !actor_enemy {
                tallies
                    .entry(target.clone())
                    .or_default()
                    .dmg_taken_from_allies += amt;
            }
        }
    }

    // suspects: "enemy" whose output goes into enemies, never hurt an
    // ally, and never got hit by one -- the shape of a friendly pet
    let mut suspects: Vec<(&String, &Tally)> = tallies
        .iter()
        .filter(|(_, t)| {
            t.dmg_into_enemies > 0 && t.dmg_into_allies == 0 && t.dmg_taken_from_allies == 0
        })
        .collect();
    suspects.sort_by_key(|(_, t)| std::cmp::Reverse(t.dmg_into_enemies));

    let total_enemy_actors = tallies.len();
    let suspect_dmg: u64 = suspects.iter().map(|(_, t)| t.dmg_into_enemies).sum();
    println!(
        "enemy-classified actors in your fights: {}; suspects (all damage into enemies, none into/from allies): {}; suspect damage mislabeled enemy-side: {}",
        total_enemy_actors,
        suspects.len(),
        suspect_dmg
    );
    println!("\ntop suspects:");
    for (name, t) in suspects.iter().take(25) {
        println!(
            "  {:<30} dmg->enemies={:<8} on-anchor={:<8} fights={}",
            name,
            t.dmg_into_enemies,
            t.hits_on_anchor,
            t.fights.len()
        );
    }

    // context: how many pets DID the inference catch
    println!(
        "\npets matched by inference (Inner Fire path): {}",
        ing.pet_owner_count()
    );
    let confirmed: Vec<&str> = ing.inferred_pets().map(|(p, _)| p).collect();
    println!(
        "confirmed pet names (sample): {:?}",
        &confirmed[..confirmed.len().min(30)]
    );

    // learn/validate the generated-name shape: syllable-built, specific
    // first letters, specific endings -- test a candidate matcher against
    // (a) confirmed pets, (b) the suspects, (c) ally-side actors (must not match)
    let shape = |n: &str| -> bool {
        let n = n.trim();
        if !n.chars().all(|c| c.is_ascii_alphabetic()) || n.len() < 4 || n.len() > 10 {
            return false;
        }
        let first_ok = matches!(
            n.chars().next(),
            Some('G' | 'J' | 'K' | 'L' | 'V' | 'X' | 'Z')
        );
        let lower = n.to_lowercase();
        let end_ok = ["n", "er", "tik", "ab"].iter().any(|e| lower.ends_with(e));
        first_ok && end_ok
    };
    let conf_match = confirmed.iter().filter(|n| shape(n)).count();
    let susp_match = suspects.iter().filter(|(n, _)| shape(n)).count();
    println!(
        "shape matcher: {}/{} confirmed pets match; {}/{} suspects match",
        conf_match,
        confirmed.len(),
        susp_match,
        suspects.len()
    );
    // false-positive check: ally-side damage actors that would shape-match
    let mut player_hits: std::collections::HashSet<String> = Default::default();
    let mut player_total: std::collections::HashSet<String> = Default::default();
    for i in 0..ing.store.len() {
        if ing.store.kind[i] != EventKind::Damage {
            continue;
        }
        let a = ing.store.name(ing.store.actor[i]).to_string();
        if !ing.allegiance_at(&a, ing.store.ts[i]).is_enemy() && !a.eq_ignore_ascii_case("you") {
            if shape(&a) {
                player_hits.insert(a.clone());
            }
            player_total.insert(a);
        }
    }
    println!(
        "false-positive check: {}/{} distinct ally-side damage actors shape-match: {:?}",
        player_hits.len(),
        player_total.len(),
        player_hits.iter().take(10).collect::<Vec<_>>()
    );

    // gap between a shape-matching suspect's FIRST action and the nearest
    // preceding summon line -- what window would actually catch them
    // PetSummon actions don't land in the store; re-scan raw lines instead
    let mut summons: Vec<i64> = Vec::new();
    for l in &lines {
        if l.windows(10).any(|w| w == b" summons a") {
            if let Some(ts) = {
                // reuse the mezz-pass parser shape
                let s = std::str::from_utf8(l.get(1..25).unwrap_or(b"")).ok();
                s.and_then(|_| super_parse_ts(l))
            } {
                summons.push(ts);
            }
        }
    }
    summons.sort_unstable();
    let mut first_seen: HashMap<String, i64> = HashMap::new();
    for e in &encs {
        for i in e.range() {
            if ing.store.enc[i] != e.id.0 || ing.store.kind[i] != EventKind::Damage {
                continue;
            }
            let actor = ing.store.name(ing.store.actor[i]).to_string();
            first_seen.entry(actor).or_insert(ing.store.ts[i]);
        }
    }
    let mut gaps: Vec<i64> = suspects
        .iter()
        .filter(|(n, _)| shape(n))
        .filter_map(|(n, _)| {
            let first = *first_seen.get(*n)?;
            let idx = summons.partition_point(|&s| s <= first);
            idx.checked_sub(1).map(|j| first - summons[j])
        })
        .collect();
    gaps.sort_unstable();
    if !gaps.is_empty() {
        let pct = |p: f64| gaps[((gaps.len() - 1) as f64 * p) as usize] / 1000;
        println!(
            "shape-matching suspects' first-action gap after nearest prior summon: n={} p25={}s p50={}s p75={}s p90={}s",
            gaps.len(),
            pct(0.25),
            pct(0.5),
            pct(0.75),
            pct(0.9)
        );
        for w in [20i64, 45, 90, 180, 600] {
            println!(
                "  window {}s would catch {} of {}",
                w,
                gaps.iter().filter(|&&g| g <= w * 1000).count(),
                gaps.len()
            );
        }
    }
}

/// same fixed-offset parse as reset_check's probe
fn super_parse_ts(line: &[u8]) -> Option<i64> {
    let s = std::str::from_utf8(line.get(1..25)?).ok()?;
    let mon = match &s[4..7] {
        "Jan" => 1,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12,
        _ => return None,
    };
    let day: i64 = s[8..10].trim().parse().ok()?;
    let h: i64 = s[11..13].parse().ok()?;
    let m: i64 = s[14..16].parse().ok()?;
    let sec: i64 = s[17..19].parse().ok()?;
    let year: i64 = s[20..24].parse().ok()?;
    let y = if mon <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (mon + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some(((days * 24 + h) * 60 + m) * 60_000 + sec * 1000)
}
