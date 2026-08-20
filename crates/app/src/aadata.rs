//! Wires the scraped AA (Alternate Advancement) catalog (`packs/aa.json`)
//! into the live app -- same `include_str!`-at-compile-time pattern
//! `itemdata.rs`/`classdata.rs`/`monsterdata.rs` already use.
//!
//! Looked up by name to attach category/description context to real log
//! grants (`ingest::AaLog`, from "You have gained the ability ..."/"You
//! have improved ..." lines) that the log itself never carries. Lookup is
//! best-effort, not exhaustive: cross-checked against the real reference
//! log's 63 distinct "gained the ability" names and 60 distinct "improved"
//! names, a handful of each had no match here -- toggle variants the log
//! reports as two separate abilities ("Spell Casting Subtlety: Disabled"/
//! "...: Enabled") that the wiki only documents once under the bare name,
//! plus a couple of AAs ("Banestrike", "Full Potential") the scrape simply
//! doesn't have pages for yet. `aa_by_name` reports a miss as `None`, the
//! same way an unrecognized item or zone does elsewhere in this app --
//! never a reason to drop the underlying log grant itself, which
//! `crate::ingest::AaLog` records regardless of whether the catalog
//! recognizes the name.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::OnceLock;

const AA_DATA_JSON: &str = include_str!("../../../packs/aa.json");

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Aa {
    pub name: String,
    /// A class name ("Wizard"), or "general"/"archetype" for AAs every
    /// class (or every caster/melee/etc. archetype) can take.
    pub category: String,
    pub ranks: u32,
    /// The wiki's own per-rank cost string ("2/4/6/9"), kept raw rather
    /// than parsed into the scrape's own structured `per_rank` (which
    /// this struct doesn't carry -- nothing here needs the level-gating
    /// detail it adds beyond what `cost_raw` already shows; the full
    /// per-rank breakdown is still on disk in `packs/aa.json` if a future
    /// need for it turns up).
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

/// Every AA the scrape carries. Parses the embedded JSON once, on first
/// use -- a malformed embedded file is a build-time data bug, loud and
/// immediate, same stance `itemdata::items` takes on its own pack.
pub fn aas() -> &'static [Aa] {
    AAS.get_or_init(|| {
        let doc: AaDoc = serde_json::from_str(AA_DATA_JSON)
            .unwrap_or_else(|e| panic!("packs/aa.json failed to parse: {e}"));
        doc.aas
    })
    .as_slice()
}

// `or_insert` (keep the *first* occurrence), not a plain collect -- two
// real names are legitimately duplicated in the catalog ("Divine Aura",
// "Quick Evacuation"), each once per class that gets its own version.
// A `HashMap::from_iter`-style collect would let whichever occurs *last*
// in the source JSON silently overwrite the other with no warning, and
// which one that is would depend on scrape/file order rather than
// anything meaningful. `or_insert` at least makes it deterministic
// (first-in-file wins) rather than silently order-dependent -- it does
// NOT resolve the real ambiguity, since a bare name lookup has no way to
// know which class's copy the log line actually meant. See `aa_by_name`.
fn index() -> &'static HashMap<String, usize> {
    AA_BY_NAME.get_or_init(|| {
        let mut m = HashMap::new();
        for (i, a) in aas().iter().enumerate() {
            m.entry(a.name.clone()).or_insert(i);
        }
        m
    })
}

/// Catalog lookup by the exact name a log line carries -- `None` for a
/// real AA the scrape doesn't have (see module doc), not an error.
///
/// Two real names are ambiguous ("Divine Aura", "Quick Evacuation" --
/// each exists once per class that has its own version), and a log
/// line's bare name carries no way to tell which one a given grant
/// actually was. This returns *a* real entry for those two (first-in-file,
/// see `index`'s doc), not the guaranteed-correct one -- category/cost/
/// description for those two specific names should be treated as
/// approximate, unlike every other entry in the catalog.
pub fn aa_by_name(name: &str) -> Option<&'static Aa> {
    index().get(name).map(|&i| &aas()[i])
}

/// Best-effort keyword match from an AA's free-text description to the
/// Character sheet's own stat rows -- eqlwiki's AA table has no
/// structured "this AA affects X" field, only prose, so this is a
/// heuristic cross-link, not a guarantee: it can miss an AA that affects
/// a stat in wording this list doesn't anticipate, and (much less likely,
/// since every phrase below was picked to be specific rather than a bare
/// word) could over-match one that happens to share a phrase without
/// really being about that stat. Every phrase was checked against a real
/// sample of AA descriptions before being added (see the module's own
/// test), not guessed blind. Shown on the Character > AA subpage as "may
/// affect" -- never folded into any computed total, since a magnitude
/// isn't extractable from prose this varied without a much larger,
/// per-AA-hand-verified effort.
// "Maximum health" alone is deliberately excluded: the one real
// description carrying that exact phrase ("First Aid") uses it for a
// bind-wound threshold, not the HP stat itself ("increases the maximum
// health you can bind wound to..."). "Maximum base health" is what
// "Natural Durability" (the AA that's actually about the HP pool) uses,
// so that's the phrase kept -- picked for being the more specific real
// phrase, not a special case carved out for First Aid.
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

// A resist type ("magic", "fire", ...) only counts if "resist" itself
// also shows up somewhere in the same description -- gates out AAs that
// mention one of these six words in an unrelated sense (a class name, a
// damage type in a completely different context) without ever being
// about resistance at all. Real example this depends on: "Innate Spell
// Resistance" reads "...improves your cold, disease, fire, magic, and
// poison resistances by..." -- the resist types share one trailing
// "resistances", not "cold resistance, disease resistance, ..." each
// spelled out, so they're matched as independent words gated on the
// shared trigger rather than as fixed two-word phrases.
const RESIST_TYPES: &[(&str, &str)] = &[
    ("SV Magic", "magic"),
    ("SV Fire", "fire"),
    ("SV Cold", "cold"),
    ("SV Disease", "disease"),
    ("SV Poison", "poison"),
    ("SV Void", "void"),
];

// Every match is scoped to one sentence, and only a sentence that itself
// describes a grant (contains one of these verb stems) is even scanned --
// real example this exists for: "Combat Agility" reads "...increases your
// melee avoidance... Melee avoidance is the component of armor class that
// allows you to avoid incoming attacks and is derived from agility, item
// avoidance, and your defense skill." The word "agility" is right there,
// but that second sentence never grants anything -- it's explaining what
// melee avoidance *is*, and has none of these verbs, so it's skipped and
// "Agi" is correctly never tagged for what's actually an AC-only AA.
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

/// One AA's effect on spell mana cost or cast time -- extracted from
/// prose, not a guaranteed-numeric field the catalog carries natively
/// (nothing here does). Real examples this is built and tested against:
/// "Spell Casting Mastery" ("...reduces the mana cost of all spells by
/// 2/5/10%..."), "Quick Damage" ("...reduces the base cast time of direct
/// damage spells that have an initial cast time of 3 seconds or more by
/// 2/5/10%..."). `per_rank_pct` lines up index-for-index with the AA's own
/// rank (index 0 = rank 1), same convention `AaGrantDto::cost_progression`
/// already uses for the raw ability-point cost string.
///
/// This is extraction and organization, not a finished calculator --
/// `scope` is kept as the raw qualifying clause text ("all spells",
/// "direct damage spells that have an initial cast time of 3 seconds or
/// more") specifically because deciding whether one particular spell
/// falls inside that clause needs real interpretation (spell type, cast
/// time, whether it has a duration, ...) that this function doesn't
/// attempt. A caller wanting an actual adjusted mana cost for a specific
/// spell still has to match `scope` against that spell's own data itself.
#[derive(Debug, Clone, Serialize)]
pub struct CostModifier {
    /// "mana_cost_pct" or "cast_time_pct".
    pub kind: String,
    pub scope: String,
    pub per_rank_pct: Vec<f64>,
}

/// Every mana-cost or cast-time reduction `aa`'s own description states,
/// gated to sentences that actually say "reduces" -- see `CostModifier`'s
/// own doc. Empty for the overwhelming majority of AAs (142 entries, only
/// a handful describe a spell-cost effect at all), which is the honest
/// answer, not a gap to fill in: most AAs simply don't affect spell cost.
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

/// `sentence` = "...<trigger><scope> by N[/N[/N...]]%...". Finds `trigger`,
/// then the *last* " by " before the next "%" (not the first -- `scope`
/// itself can legitimately contain other instances of the word, and the
/// one immediately before the percentage list is always the real
/// separator). `None` if `trigger` isn't in this sentence, or what
/// follows doesn't parse as a percent clause at all.
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

    /// Cross-checked against the real reference log's own "gained the
    /// ability" names -- most should resolve; a few known misses (toggle
    /// variants, missing scrape pages) are allowed. If this regresses to
    /// zero matches, the catalog or the log's naming has drifted.
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

    /// `relevant_stats` against real AA descriptions -- picked because
    /// each exercises a different part of the matcher: a listed-stats
    /// phrase (Innate Eminence), the resist-trigger-plus-word-list gate
    /// (Innate Spell Resistance, which must NOT tag SV Void -- the
    /// description never mentions it), a two-word AC phrase under a
    /// different name ("melee avoidance"), and a description that should
    /// tag nothing at all (a pure combat-mechanic AA, no stat-sheet
    /// row involved).
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

    /// `cost_modifiers` against the three real descriptions it was built
    /// from: an unconditional mana-cost reduction, a scoped cast-time
    /// reduction, and one whose scope clause contains the word "and"
    /// (making sure the scope text is captured whole, not cut short).
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

    /// Most AAs don't touch spell cost at all -- confirms that's an empty
    /// result, not a false match off some unrelated "reduces...by N%"
    /// phrasing (e.g. Stoicism reduces knockback distance, not a spell
    /// cost).
    #[test]
    fn cost_modifiers_is_empty_for_an_unrelated_aa() {
        let stoicism = aa_by_name("Stoicism").expect("real catalog entry");
        assert!(cost_modifiers(stoicism).is_empty());
    }

    /// The two real duplicate-named entries still resolve to *something*
    /// real (see `index`'s own doc for why picking a specific one of the
    /// two is inherently ambiguous, not a bug this test is pretending to
    /// fix).
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
