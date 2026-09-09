//! why: Debug > Parsed -- every in-memory table behind one regex, newest
//! first, scan cut at the limit. The rendered row is the contract a
//! custom trigger will match against later, so its shape stays put.

use crate::ingest::Ingest;
use eqlp_source::Millis;
use eqlp_store::{flag, tag, Flags, Sym, NO_ENCOUNTER};
use regex::{Regex, RegexBuilder};
use serde::Serialize;
use serde_json::{json, Value};

pub const TABLES: &[&str] = &[
    "events",
    "encounters",
    "timeline",
    "zones",
    "units",
    "levels",
    "party",
    "entities",
    "effects",
    "self_buffs",
    "chat",
    "pms",
    "spellbook",
    "spell_ranks",
    "spell_perf",
    "exaltations",
    "aa",
    "aa_points",
    "skills",
    "skill_levels",
    "crafts",
    "tradeskill_levels",
    "turn_ins",
    "achievements",
    "disposed",
    "drops",
    "zone_drops",
    "instances",
    "names",
    "abilities",
    "counts",
];

#[derive(Debug, Clone, Serialize)]
pub struct SearchDbDto {
    pub tables: Vec<&'static str>,
    pub table: String,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
    /// why: rows looked at -- below total when the limit cut the scan
    pub scanned: usize,
    pub matched: usize,
    pub total: usize,
    pub truncated: bool,
}

/// why: empty pattern browses; the regex is case-insensitive and runs over
/// the row's cells joined by " | ", so a date, a name or a flag all match
pub fn search(
    ing: &Ingest,
    table: &str,
    pattern: &str,
    limit: usize,
) -> Result<SearchDbDto, String> {
    let re = if pattern.is_empty() {
        None
    } else {
        Some(
            RegexBuilder::new(pattern)
                .case_insensitive(true)
                .size_limit(1 << 20)
                .build()
                .map_err(|e| e.to_string())?,
        )
    };
    if table == "events" {
        let (columns, total, rows) = events(ing);
        return Ok(collect(table, columns, total, rows, re.as_ref(), limit));
    }
    let values = table_values(ing, table).ok_or_else(|| format!("no table named {table}"))?;
    let (columns, rows) = from_values(values);
    let total = rows.len();
    Ok(collect(
        table,
        columns,
        total,
        rows.into_iter(),
        re.as_ref(),
        limit,
    ))
}

fn collect(
    table: &str,
    columns: Vec<String>,
    total: usize,
    rows: impl Iterator<Item = Vec<String>>,
    re: Option<&Regex>,
    limit: usize,
) -> SearchDbDto {
    let mut out = Vec::new();
    let mut scanned = 0;
    for cells in rows {
        scanned += 1;
        if re.is_none_or(|r| r.is_match(&cells.join(" | "))) {
            out.push(cells);
            if out.len() >= limit {
                break;
            }
        }
    }
    SearchDbDto {
        tables: TABLES.to_vec(),
        table: table.to_string(),
        columns,
        matched: out.len(),
        rows: out,
        scanned,
        total,
        truncated: scanned < total,
    }
}

/// why: the store is scanned lazily, newest first -- a common pattern
/// stops after a few thousand rows instead of rendering 700k
fn events(ing: &Ingest) -> (Vec<String>, usize, impl Iterator<Item = Vec<String>> + '_) {
    let s = &ing.store;
    let columns = [
        "ts", "kind", "actor", "target", "ability", "tags", "amount", "flags", "enc", "tier",
        "rows",
    ]
    .iter()
    .map(|c| c.to_string())
    .collect();
    let rows = (0..s.len()).rev().map(move |i| {
        let ab = s.abilities.get(s.ability[i]);
        vec![
            civil(s.ts[i]),
            format!("{:?}", s.kind[i]).to_ascii_lowercase(),
            s.name(s.actor[i]).to_string(),
            s.name(s.target[i]).to_string(),
            ab.map(|a| s.name(a.name).to_string()).unwrap_or_default(),
            ab.map(|a| tag::names(a.tags).join("+")).unwrap_or_default(),
            s.amount[i].to_string(),
            flag_names(s.flags[i]),
            if s.enc[i] == NO_ENCOUNTER {
                String::new()
            } else {
                s.enc[i].to_string()
            },
            s.tier[i].to_string(),
            s.count[i].to_string(),
        ]
    });
    (columns, s.len(), rows)
}

fn flag_names(f: Flags) -> String {
    const ALL: &[(Flags, &str)] = &[
        (flag::CRITICAL, "crit"),
        (flag::RIPOSTE, "riposte"),
        (flag::RAMPAGE, "rampage"),
        (flag::STRIKETHROUGH, "strikethrough"),
        (flag::CRIPPLING, "crippling"),
        (flag::FINISHING, "finishing"),
        (flag::SLAY_UNDEAD, "slay_undead"),
        (flag::DOUBLE_BOW, "double_bow"),
        (flag::FLURRY, "flurry"),
        (flag::LOOT_AUTO_SOLD, "auto_sold"),
    ];
    let mut names: Vec<String> = ALL
        .iter()
        .filter(|(bit, _)| f & bit != 0)
        .map(|(_, n)| n.to_string())
        .collect();
    let known: Flags = ALL.iter().fold(0, |acc, (bit, _)| acc | bit);
    if f & !known != 0 {
        names.push(format!("0x{:x}", f & !known));
    }
    names.join("+")
}

fn vals<T: Serialize>(v: Vec<T>) -> Vec<Value> {
    v.into_iter()
        .filter_map(|x| serde_json::to_value(x).ok())
        .collect()
}

/// why: one arm per table; DTO-backed tables reuse the view's own function
/// so the search never disagrees with the module that shows the same data
fn table_values(ing: &Ingest, table: &str) -> Option<Vec<Value>> {
    let s = &ing.store;
    let name = |e: u32| s.name(Sym(e)).to_string();
    Some(match table {
        "encounters" => vals(crate::debugview::list_debug_encounters(ing, usize::MAX)),
        "timeline" => ing
            .timeline
            .between(Millis::MIN, Millis::MAX)
            .iter()
            .map(|t| {
                json!({"ts": t.ts, "entity": name(t.entity), "state": format!("{:?}", t.state), "cause": format!("{:?}", t.cause)})
            })
            .collect(),
        "zones" => vals(crate::combat::list_zone_visits(ing)),
        "units" => (0..ing.units.len())
            .filter_map(|i| {
                ing.units
                    .bounds(i)
                    .map(|(a, b)| json!({"unit": i, "start_ms": a, "end_ms": b}))
            })
            .collect(),
        "levels" => ing
            .levels
            .all()
            .iter()
            .map(|(ts, l)| json!({"ts": ts, "level": l}))
            .collect(),
        "party" => vals(crate::debugview::game_state(ing).party),
        "entities" => ing
            .encounters
            .entities
            .all()
            .map(|(n, k, o)| json!({"name": n, "kind": format!("{k:?}"), "owner": o}))
            .collect(),
        "effects" => ing
            .effects
            .iter_all()
            .map(|(e, p)| json!({"ts": p.ts, "entity": name(e), "text": &*p.text}))
            .collect(),
        "self_buffs" => ing
            .self_buffs
            .iter()
            .map(|(n, (since, until))| json!({"buff": n, "since_ms": since, "until_ms": until}))
            .collect(),
        "chat" => [
            ("guild", crate::chat::guild_chat(ing)),
            ("party", crate::chat::party_chat(ing)),
            ("raid", crate::chat::raid_chat(ing)),
        ]
        .into_iter()
        .flat_map(|(ch, msgs)| {
            msgs.into_iter().map(move |m| {
                json!({"ts_ms": m.ts_ms, "channel": ch, "who": m.who, "text": m.text})
            })
        })
        .collect(),
        "pms" => crate::chat::pm_threads(ing)
            .into_iter()
            .flat_map(|t| {
                let hist = crate::chat::pm_history(ing, &t.player);
                hist.into_iter().map(move |m| {
                    json!({"ts_ms": m.ts_ms, "thread": t.player, "who": m.who, "text": m.text})
                })
            })
            .collect(),
        "spellbook" => vals(crate::progression::spellbook(ing)),
        "spell_ranks" => ing
            .spell_ranks
            .all()
            .map(|(sp, r)| json!({"spell": sp, "rank": r}))
            .collect(),
        "spell_perf" => ing
            .spell_perf
            .all()
            .map(|(sp, st)| {
                json!({"spell": sp, "landings": st.landings, "ema_recent": st.ema_recent, "ema_norm": st.ema_norm, "recent_n": st.recent_n, "last_ms": st.last_ms})
            })
            .collect(),
        "exaltations" => ing
            .exaltation_procs
            .all()
            .map(|(item, n, first)| json!({"item": item, "procs": n, "first_seen_ms": first}))
            .collect(),
        "aa" => ing
            .aa
            .all()
            .map(|(ts, g)| json!({"ts": ts, "name": g.name, "rank": g.rank, "cost": g.cost}))
            .collect(),
        "aa_points" => ing
            .aa_points
            .iter()
            .map(|(ts, g, t)| json!({"ts": ts, "gained": g, "total": t}))
            .collect(),
        "skills" => vals(crate::skilltracker::skill_status(ing)),
        "skill_levels" => ing
            .skill_levels
            .iter()
            .map(|(sk, (lvl, ts))| json!({"skill": sk, "level": lvl, "ts": ts}))
            .collect(),
        "crafts" => vals(crate::craftlog::craft_log(ing)),
        "tradeskill_levels" => vals(crate::craftlog::tradeskill_levels(ing)),
        "turn_ins" => ing
            .turn_ins
            .iter()
            .map(|t| {
                let items: Vec<String> = t.items.iter().map(|(i, n)| format!("{i} ×{n}")).collect();
                json!({"ts": t.ts, "who": t.who, "items": items})
            })
            .collect(),
        "achievements" => ing
            .achievements_live
            .iter()
            .map(|a| json!({"achievement": a}))
            .collect(),
        "disposed" => ing
            .disposed_items
            .iter()
            .map(|i| json!({"item": i}))
            .collect(),
        "drops" => ing
            .observed_drops
            .iter()
            .flat_map(|(mob, items)| items.iter().map(move |i| json!({"mob": mob, "item": i})))
            .collect(),
        "zone_drops" => ing
            .observed_zone_drops
            .iter()
            .flat_map(|(z, items)| items.iter().map(move |i| json!({"zone": z, "item": i})))
            .collect(),
        "instances" => ing
            .instances
            .iter()
            .map(|(sym, (n, ts))| json!({"name": s.name(*sym), "count": n, "ts": ts}))
            .collect(),
        "names" => (0..s.names.len())
            .map(|i| json!({"sym": i, "name": s.names.name(Sym(i as u32))}))
            .collect(),
        "abilities" => s
            .abilities
            .iter()
            .map(|(id, a)| json!({"id": id.0, "name": s.name(a.name), "tags": tag::names(a.tags)}))
            .collect(),
        "counts" => ing
            .counts
            .by_kind
            .iter()
            .map(|(k, n)| json!({"kind": k, "lines": n}))
            .chain([
                json!({"kind": "(total)", "lines": ing.counts.total}),
                json!({"kind": "(matched)", "lines": ing.counts.matched}),
                json!({"kind": "(unmatched)", "lines": ing.counts.unmatched}),
            ])
            .collect(),
        _ => return None,
    })
}

const TS_KEYS: &[&str] = &[
    "ts",
    "ts_ms",
    "start_ms",
    "last_ms",
    "first_seen_ms",
    "last_ts_ms",
    "since_ms",
    "last_used_ms",
    "latest_ms",
];

/// why: columns are the union of keys, the timestamp first; rows sort
/// newest first on it, or by the first cell when the table has no clock
fn from_values(values: Vec<Value>) -> (Vec<String>, Vec<Vec<String>>) {
    let mut keys: Vec<String> = Vec::new();
    for v in &values {
        if let Value::Object(m) = v {
            for k in m.keys() {
                if !keys.contains(k) {
                    keys.push(k.clone());
                }
            }
        }
    }
    let ts_key = TS_KEYS
        .iter()
        .find(|k| keys.iter().any(|x| x == *k))
        .copied();
    keys.sort_by_key(|k| (Some(k.as_str()) != ts_key, k.clone()));
    let mut rows: Vec<(Millis, Vec<String>)> = values
        .iter()
        .map(|v| {
            let m = v.as_object();
            let ts = ts_key
                .and_then(|k| m.and_then(|m| m.get(k)))
                .and_then(Value::as_i64)
                .unwrap_or(0);
            let cells = keys
                .iter()
                .map(|k| render(k, m.and_then(|m| m.get(k))))
                .collect();
            (ts, cells)
        })
        .collect();
    if ts_key.is_some() {
        rows.sort_by_key(|r| std::cmp::Reverse(r.0));
    } else {
        rows.sort_by_cached_key(|r| r.1.first().cloned().unwrap_or_default());
    }
    (keys, rows.into_iter().map(|r| r.1).collect())
}

/// why: a millisecond clock reads as a date; everything else as itself
fn render(key: &str, v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => match n.as_i64() {
            Some(ms) if ms >= 100_000_000_000 && (key == "ts" || key.ends_with("_ms")) => civil(ms),
            _ => n.to_string(),
        },
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Array(a)) => a
            .iter()
            .map(|x| render("", Some(x)))
            .collect::<Vec<_>>()
            .join(", "),
        Some(other) => other.to_string(),
    }
}

/// why: log time is wall-clock read as UTC (core/header.rs), so the date
/// comes straight back out with no zone -- Hinnant's civil_from_days
pub fn civil(ms: Millis) -> String {
    let secs = ms.div_euclid(1000);
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}",
        sod / 3600,
        (sod % 3600) / 60,
        sod % 60
    )
}
