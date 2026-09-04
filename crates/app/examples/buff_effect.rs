//! why: "does this buff meaningfully affect my damage?" -- measures YOUR
//!      own non-crit hits while a named buff is on you against the same
//!      spell's hits outside it, controls matched within +-15 minutes so
//!      rank, gear and target are held roughly still, and runs the same
//!      test on windows shifted +-10 minutes as a placebo. A real effect
//!      shows in the real windows and vanishes in the placebos.
//! input: <log> "<buff spell name>"
//! run: cargo run -p eqlp-app --release --example buff_effect -- <log> "Rizlona's Embers"

use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;
use eqlp_source::Millis;
use eqlp_store::{flag, EventKind};

/// why: matched controls -- the same spell, cast around the same time
const CONTROL_WINDOW_MS: Millis = 15 * 60 * 1000;
/// why: enough hits to say anything at all
const MIN_HITS: usize = 25;

/// why: read from the raw lines, not from Effects -- the app's ping log
/// is a live view (the zone cull trims it), and this question is about
/// the whole file. The buff's own first-person landing text is the mark.
fn buff_windows(lines: &[&[u8]], landing: &str, dur_ms: Millis) -> Vec<(Millis, Millis)> {
    let header = eqlp_core::header::by_name("bracket-ctime").expect("header");
    let mut w: Vec<(Millis, Millis)> = lines
        .iter()
        .filter_map(|l| {
            let (ts, off) = header.parse(l)?;
            let body = std::str::from_utf8(&l[off..]).ok()?.trim();
            (body == landing).then(|| {
                let t = ts.secs() * 1000;
                (t, t + dur_ms)
            })
        })
        .collect();
    w.sort();
    let mut merged: Vec<(Millis, Millis)> = Vec::new();
    for (a, b) in w {
        match merged.last_mut() {
            Some(last) if a <= last.1 => last.1 = last.1.max(b),
            _ => merged.push((a, b)),
        }
    }
    merged
}

/// why: your own landed, non-critical SPELL hits -- (ts, ability,
/// amount). Melee swings and damage-shield procs are excluded: this
/// class of buff limits itself to spells ("Combat Skills Not Allowed"),
/// and a shield's own damage tracks the shield's rank, not yours, so
/// leaving them in swamped the pooled number with unrelated variance.
fn my_hits(ing: &Ingest) -> Vec<(Millis, String, u64)> {
    let Some(you) = ing.store.names.get("You") else {
        return Vec::new();
    };
    (0..ing.store.len())
        .filter(|&i| {
            ing.store.kind[i] == EventKind::Damage
                && ing.store.actor[i] == you
                && ing.store.flags[i] & flag::CRITICAL == 0
                && ing.store.flags[i] & (flag::MISSED | flag::BLOCKED | flag::DODGED | flag::PARRIED)
                    == 0
                && ing.store.count[i] == 1 // why: never a folded row
                && ing.store.abilities.tags(ing.store.ability[i]) & eqlp_store::tag::SPELL != 0
                && !ing
                    .store
                    .ability_name(ing.store.ability[i])
                    .starts_with("Damage Shield")
        })
        .map(|i| {
            (
                ing.store.ts[i],
                ing.store.ability_name(ing.store.ability[i]).to_string(),
                ing.store.amount[i],
            )
        })
        .collect()
}

fn on(windows: &[(Millis, Millis)], t: Millis) -> bool {
    match windows.binary_search_by(|w| w.0.cmp(&t)) {
        Ok(_) => true,
        Err(0) => false,
        Err(i) => t <= windows[i - 1].1,
    }
}

/// why: one spell's buffed hits against time-local controls; the ratio
/// per hit, so a rank change inside the log cannot fake a result
fn ratios(hits: &[(Millis, String, u64)], windows: &[(Millis, Millis)]) -> Vec<(String, Vec<f64>)> {
    /// why: one ability's hits, split by whether the buff was up
    type Split = (Vec<(Millis, u64)>, Vec<(Millis, u64)>);
    let mut by: std::collections::HashMap<String, Split> = std::collections::HashMap::new();
    for (t, ability, amt) in hits {
        let e = by.entry(ability.clone()).or_default();
        if on(windows, *t) {
            e.0.push((*t, *amt));
        } else {
            e.1.push((*t, *amt));
        }
    }
    let mut out = Vec::new();
    for (ability, (buffed, plain)) in by {
        if buffed.len() < MIN_HITS || plain.len() < MIN_HITS {
            continue;
        }
        let times: Vec<Millis> = plain.iter().map(|(t, _)| *t).collect();
        let mut rs = Vec::new();
        for (t, amt) in &buffed {
            let lo = times.partition_point(|x| *x < t - CONTROL_WINDOW_MS);
            let hi = times.partition_point(|x| *x <= t + CONTROL_WINDOW_MS);
            if hi - lo < 5 {
                continue;
            }
            let mean: f64 =
                plain[lo..hi].iter().map(|(_, a)| *a as f64).sum::<f64>() / (hi - lo) as f64;
            if mean > 0.0 {
                rs.push(*amt as f64 / mean);
            }
        }
        if rs.len() >= MIN_HITS {
            out.push((ability, rs));
        }
    }
    out.sort_by_key(|(_, rs)| std::cmp::Reverse(rs.len()));
    out
}

/// why: a percentile interval from resampling -- no distribution assumed
fn ci(rs: &[f64]) -> (f64, f64) {
    let mut seed = 0x5eed_u64;
    let mut means: Vec<f64> = (0..2000)
        .map(|_| {
            let s: f64 = (0..rs.len())
                .map(|_| {
                    seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                    rs[(seed >> 33) as usize % rs.len()]
                })
                .sum();
            s / rs.len() as f64
        })
        .collect();
    means.sort_by(|a, b| a.partial_cmp(b).unwrap());
    (means[50], means[1949])
}

fn report(label: &str, hits: &[(Millis, String, u64)], windows: &[(Millis, Millis)]) {
    println!("== {label} ({} windows)", windows.len());
    let rows = ratios(hits, windows);
    let mut pooled: Vec<f64> = Vec::new();
    for (ability, rs) in &rows {
        let mean = rs.iter().sum::<f64>() / rs.len() as f64;
        let (lo, hi) = ci(rs);
        println!(
            "  {:30} n={:4}  {:+6.1}%  95% CI [{:+5.1}%, {:+5.1}%]",
            ability,
            rs.len(),
            100.0 * (mean - 1.0),
            100.0 * (lo - 1.0),
            100.0 * (hi - 1.0)
        );
        pooled.extend(rs.iter().copied());
    }
    if !pooled.is_empty() {
        let mean = pooled.iter().sum::<f64>() / pooled.len() as f64;
        let (lo, hi) = ci(&pooled);
        println!(
            "  {:30} n={:4}  {:+6.1}%  95% CI [{:+5.1}%, {:+5.1}%]",
            "ALL POOLED",
            pooled.len(),
            100.0 * (mean - 1.0),
            100.0 * (lo - 1.0),
            100.0 * (hi - 1.0)
        );
    }
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let buff = a
        .get(1)
        .cloned()
        .unwrap_or_else(|| "Rizlona's Embers".into());
    let raw = std::fs::read(&a[0]).expect("log");
    let lines = framed_lines(&raw);
    let engine = build_engine().expect("pack");
    let mut ing = Ingest::default();
    // why: the zone cull folds old rows and drops old pings; this
    // question is about the whole log, so keep it all
    ing.keep_full_history = true;
    for chunk in lines.chunks(100_000) {
        backfill_lines(&mut ing, &engine, chunk, 8);
    }
    let spell = eqlp_app::spelldata::spell_by_name(&buff).expect("buff in the catalog");
    let d = eqlp_app::spelleffect::parse_duration(spell.duration.as_deref());
    let dur_ms = (d.max_secs.or(d.min_secs).unwrap_or(12.0) * 1000.0) as Millis;
    println!(
        "{buff}: {:?}, counted for {dur_ms}ms per landing",
        spell.duration
    );
    for slot in &spell.slots {
        println!("  {}", slot.effect);
    }
    let hits = my_hits(&ing);
    let landing = spell
        .msg_cast_on_you
        .clone()
        .expect("the buff has a first-person landing message");
    println!("landing text: {landing:?}");
    let windows = buff_windows(&lines, &landing, dur_ms);
    report("real windows", &hits, &windows);
    let shift = 10 * 60 * 1000;
    report(
        "placebo, shifted +10 min",
        &hits,
        &windows
            .iter()
            .map(|(a, b)| (a + shift, b + shift))
            .collect::<Vec<_>>(),
    );
    report(
        "placebo, shifted -10 min",
        &hits,
        &windows
            .iter()
            .map(|(a, b)| (a - shift, b - shift))
            .collect::<Vec<_>>(),
    );
}
