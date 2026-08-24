//! why: AA catalog, baked in like `itemdata.rs`/`classdata.rs`/`monsterdata.rs`
//!
//! Attaches category/description context to real log grants (`ingest::
//! AaLog`) the log itself never carries. Best-effort, not exhaustive --
//! cross-checked against 63/60 real distinct gained/improved names, a
//! handful miss (toggle variants, unscraped pages). Miss reports None,
//! never drops the underlying log grant itself.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::OnceLock;

const AA_DATA_JSON: &str = include_str!("../../../packs/aa.json");

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Aa {
    pub name: String,
    /// why: a class name, or "general"/"archetype" for every-class AAs
    pub category: String,
    pub ranks: u32,
    /// why: wiki's raw per-rank cost string, kept raw not parsed into
    /// the fuller `per_rank` structure still on disk in `packs/aa.json`
    pub cost_raw: String,
    pub certain: bool,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AaDoc {
    aas: Vec<Aa>,
}

static AAS: OnceLock<Vec<Aa>> = OnceLock::new();
static AA_BY_NAME: OnceLock<HashMap<String, usize>> = OnceLock::new();

/// why: every scraped AA, parsed once; malformed data fails loud
pub fn aas() -> &'static [Aa] {
    AAS.get_or_init(|| {
        let doc: AaDoc = serde_json::from_str(AA_DATA_JSON)
            .unwrap_or_else(|e| panic!("packs/aa.json failed to parse: {e}"));
        doc.aas
    })
    .as_slice()
}

// why: `or_insert` keeps first occurrence deterministically -- 2 real
// names are legitimately duplicated per-class; doesn't resolve the
// ambiguity, just makes which copy wins deterministic. See `aa_by_name`.
fn index() -> &'static HashMap<String, usize> {
    AA_BY_NAME.get_or_init(|| {
        let mut m = HashMap::new();
        for (i, a) in aas().iter().enumerate() {
            m.entry(a.name.clone()).or_insert(i);
        }
        m
    })
}

/// why: lookup by exact log name, None for unscraped; "Divine Aura"/
/// "Quick Evacuation" are ambiguous (one per class) -- returns *a* real
/// entry, not guaranteed the right one, for just those two
pub fn aa_by_name(name: &str) -> Option<&'static Aa> {
    index().get(name).map(|&i| &aas()[i])
}

/// why: heuristic keyword match, eqlwiki AA table has no structured
/// "affects X" field, only prose. Every phrase checked against a real
/// sample first. Shown as "may affect", never folded into a computed total.
// why: "maximum health" excluded -- that exact phrase belongs to a
// bind-wound threshold (First Aid), not the HP stat; "maximum base
// health" is the real HP-pool phrase (Natural Durability)
const STAT_PHRASES: &[(&str, &[&str])] = &[
    ("HP", &["maximum base health", "maximum hit points"]),
    (
        "HP Regen",
        &["health regeneration", "hit point regeneration"],
    ),
    ("Mana", &["maximum mana"]),
    ("Mana Regen", &["mana regeneration"]),
    ("Endurance", &["maximum endurance"]),
    ("End Regen", &["endurance regeneration"]),
    ("AC", &["armor class", "melee avoidance"]),
    ("Attack", &["attack rating"]),
    ("Velocity", &["attack speed", "haste"]),
    ("Str", &["strength"]),
    ("Stam", &["stamina"]),
    ("Agi", &["agility"]),
    ("Dex", &["dexterity"]),
    ("Wis", &["wisdom"]),
    ("Int", &["intelligence"]),
    ("Cha", &["charisma"]),
];

// why: gated on "resist" appearing too, else a damage-type word alone
// could false-match; real descriptions share one trailing "resistances"
const RESIST_TYPES: &[(&str, &str)] = &[
    ("SV Magic", "magic"),
    ("SV Fire", "fire"),
    ("SV Cold", "cold"),
    ("SV Disease", "disease"),
    ("SV Poison", "poison"),
    ("SV Void", "void"),
];

// why: only a sentence with a grant verb is scanned -- excludes an
// explanatory sentence that mentions a stat word without granting it
const GRANT_VERBS: &[&str] = &["increas", "improv", "grant", "boost"];

fn word_present(haystack: &str, word: &str) -> bool {
    haystack
        .split(|c: char| !c.is_alphanumeric())
        .any(|tok| tok == word)
}

pub fn relevant_stats(description: &str) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for sentence in description.split('.') {
        let lower = sentence.to_lowercase();
        if !GRANT_VERBS.iter().any(|v| lower.contains(v)) {
            continue;
        }
        for (stat, phrases) in STAT_PHRASES {
            if phrases.iter().any(|p| lower.contains(p)) && !out.contains(stat) {
                out.push(stat);
            }
        }
        if lower.contains("resist") {
            for (stat, word) in RESIST_TYPES {
                if word_present(&lower, word) && !out.contains(stat) {
                    out.push(stat);
                }
            }
        }
    }
    out
}

/// why: mana/cast-time effect extracted from prose, not a native field.
/// `per_rank_pct` indexes 0=rank1, same convention as `cost_progression`.
/// Extraction only, not a calculator -- `scope` stays raw clause text;
/// matching it against a specific spell is left to the caller.
#[derive(Debug, Clone, Serialize)]
pub struct CostModifier {
    /// why: "mana_cost_pct" or "cast_time_pct"
    pub kind: String,
    pub scope: String,
    pub per_rank_pct: Vec<f64>,
}

/// why: gated to "reduces" sentences; empty for most of 142 AAs, the honest answer
pub fn cost_modifiers(aa: &Aa) -> Vec<CostModifier> {
    let Some(desc) = aa.description.as_deref() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for sentence in desc.split('.') {
        if !sentence.contains("reduces") {
            continue;
        }
        if let Some((scope, per_rank_pct)) = parse_pct_clause(sentence, "mana cost of") {
            out.push(CostModifier {
                kind: "mana_cost_pct".to_string(),
                scope,
                per_rank_pct,
            });
        }
        if let Some((scope, per_rank_pct)) = parse_pct_clause(sentence, "cast time of") {
            out.push(CostModifier {
                kind: "cast_time_pct".to_string(),
                scope,
                per_rank_pct,
            });
        }
    }
    out
}

/// why: finds `trigger`, then the *last* " by " before "%" -- scope can
/// legitimately contain other " by " occurrences
fn parse_pct_clause(sentence: &str, trigger: &str) -> Option<(String, Vec<f64>)> {
    let start = sentence.find(trigger)? + trigger.len();
    let rest = &sentence[start..];
    let pct_end = rest.find('%')?;
    let clause = &rest[..pct_end];
    let by_idx = clause.rfind(" by ")?;
    let scope = clause[..by_idx].trim().to_string();
    let per_rank_pct: Vec<f64> = clause[by_idx + 4..]
        .split('/')
        .filter_map(|s| s.trim().parse::<f64>().ok())
        .collect();
    if per_rank_pct.is_empty() {
        None
    } else {
        Some((scope, per_rank_pct))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_and_indexes_the_real_catalog() {
        assert_eq!(aas().len(), 142);
        let a = aa_by_name("Adamant Will").expect("Adamant Will should be in the catalog");
        assert_eq!(a.ranks, 4);
        assert_eq!(a.cost_raw, "2/4/6/9");
    }

    #[test]
    fn unknown_name_is_none_not_a_panic() {
        assert!(aa_by_name("Not A Real Ability").is_none());
    }

    /// why: cross-checked against real "gained the ability" names, most
    /// should resolve; regressing to zero means the catalog/log has drifted
    #[test]
    fn most_real_gained_names_resolve() {
        let real_names = [
            "Adamant Will",
            "Combat Agility",
            "Fear Resistance",
            "Improved Familiar",
            "Leech Touch",
            "Mnemonic Retention",
        ];
        for name in real_names {
            assert!(
                aa_by_name(name).is_some(),
                "{name} should resolve against the real catalog"
            );
        }
    }

    /// why: each case exercises a different matcher path -- stat-list
    /// phrase, resist gate, AC synonym, and a no-match combat-mechanic AA
    #[test]
    fn relevant_stats_matches_real_descriptions() {
        let eminence = aa_by_name("Innate Eminence").expect("real catalog entry");
        let mut got = relevant_stats(eminence.description.as_deref().unwrap_or(""));
        got.sort_unstable();
        assert_eq!(got, ["Agi", "Cha", "Dex", "Int", "Stam", "Str", "Wis"]);

        let resist = aa_by_name("Innate Spell Resistance").expect("real catalog entry");
        let mut got = relevant_stats(resist.description.as_deref().unwrap_or(""));
        got.sort_unstable();
        assert_eq!(
            got,
            ["SV Cold", "SV Disease", "SV Fire", "SV Magic", "SV Poison"]
        );
        assert!(
            !got.contains(&"SV Void"),
            "the description never mentions void, so it must not be tagged"
        );

        let agility = aa_by_name("Combat Agility").expect("real catalog entry");
        assert_eq!(
            relevant_stats(agility.description.as_deref().unwrap_or("")),
            vec!["AC"]
        );

        let ambidexterity = aa_by_name("Ambidexterity").expect("real catalog entry");
        assert!(
            relevant_stats(ambidexterity.description.as_deref().unwrap_or("")).is_empty(),
            "a dual-wield-chance AA has no Character sheet row to point at"
        );
    }

    /// why: 3 real descriptions -- unconditional mana reduction, scoped
    /// cast-time reduction, and a scope clause containing "and" (not cut short)
    #[test]
    fn cost_modifiers_matches_real_descriptions() {
        let mastery = aa_by_name("Spell Casting Mastery").expect("real catalog entry");
        let got = cost_modifiers(mastery);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].kind, "mana_cost_pct");
        assert_eq!(got[0].scope, "all spells");
        assert_eq!(got[0].per_rank_pct, vec![2.0, 5.0, 10.0]);

        let quick_damage = aa_by_name("Quick Damage").expect("real catalog entry");
        let got = cost_modifiers(quick_damage);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].kind, "cast_time_pct");
        assert_eq!(
            got[0].scope,
            "direct damage spells that have an initial cast time of 3 seconds or more"
        );
        assert_eq!(got[0].per_rank_pct, vec![2.0, 5.0, 10.0]);

        let deftness = aa_by_name("Spell Casting Deftness").expect("real catalog entry");
        let got = cost_modifiers(deftness);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].kind, "cast_time_pct");
        assert_eq!(
            got[0].scope,
            "beneficial spells that have a duration and an initial cast time of at least 3 seconds"
        );
        assert_eq!(got[0].per_rank_pct, vec![10.0, 25.0, 50.0]);
    }

    /// why: most AAs don't touch spell cost -- not a false "reduces...by N%" match
    #[test]
    fn cost_modifiers_is_empty_for_an_unrelated_aa() {
        let stoicism = aa_by_name("Stoicism").expect("real catalog entry");
        assert!(cost_modifiers(stoicism).is_empty());
    }

    /// why: duplicate-named entries still resolve to something real, ambiguity is inherent
    #[test]
    fn duplicate_names_resolve_to_a_real_entry_not_none() {
        for name in ["Divine Aura", "Quick Evacuation"] {
            assert!(
                aa_by_name(name).is_some(),
                "{name} should still resolve to one of its two real entries"
            );
        }
    }
}
