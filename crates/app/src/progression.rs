//! AA (Alternate Advancement) and Spellbook progression: every rank
//! purchase and every Known/Possible spell seen this session, from
//! `ingest::AaLog`/`ingest::SpellLog`, enriched (best-effort -- see
//! `crate::aadata`/`crate::spelldata`'s own docs) with the scraped
//! catalogs. The data's real: `You have gained the ability "X" at a cost
//! of N ability points.`/`You have improved X N at a cost of M ability
//! points.` are exhaustively covered (101/101 real "at a cost of" lines
//! matched in the reference log, split correctly between the two forms),
//! and both spellbook signals are covered too -- scribing a new scroll
//! (596/593 real begin/finish lines) and memorizing a gem (the original
//! signal this module used before scribing's own begin/finish pair was
//! found) -- see `ingest::SpellLog`'s own doc for exactly what "Known"
//! and "Possible" mean.

use crate::ingest::Ingest;
use eqlp_source::Millis;
use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize)]
pub struct AaGrantDto {
    pub ts_ms: Millis,
    pub name: String,
    pub rank: u8,
    pub cost: u32,
    /// Everything below is catalog enrichment -- all `None` together for
    /// the handful of real names the catalog doesn't have (see `aadata`'s
    /// module doc). Never a reason to drop the grant itself, which is
    /// real regardless of whether the catalog recognizes the name.
    pub category: Option<String>,
    pub description: Option<String>,
    /// How many ranks this AA goes to in total, so a future view can show
    /// "rank 2 of 4" rather than just the bare rank number this grant is.
    pub max_rank: Option<u32>,
    /// The wiki's own per-rank cost string ("2/4/6/9") -- shown as-is
    /// rather than re-deriving it from `per_rank`, the same "raw scrape
    /// string, kept as-is" stance `aadata::Aa::cost_raw` itself takes.
    pub cost_progression: Option<String>,
    /// `aadata::Aa::certain` -- whether the scrape is confident this
    /// entry's numbers are complete/correct, surfaced so a future UI can
    /// flag an uncertain one rather than presenting it with the same
    /// confidence as a verified entry.
    pub catalog_certain: Option<bool>,
    /// `aadata::relevant_stats` -- Character sheet stat rows this AA's own
    /// description suggests it affects, best-effort (see that function's
    /// doc). Empty for an AA with no catalog match, or one whose
    /// description doesn't hit any of the matcher's own phrases.
    pub relevant_stats: Vec<String>,
    /// `aadata::cost_modifiers` -- mana-cost/cast-time effects this AA's
    /// own description states, at every rank (not just the rank actually
    /// owned -- see that function's own doc for why `scope` is left as
    /// raw text rather than resolved against any specific spell here).
    /// Empty for the overwhelming majority of AAs, which don't touch
    /// spell cost at all.
    pub cost_modifiers: Vec<crate::aadata::CostModifier>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AaLogDto {
    /// Log-time order, oldest first -- same convention `AaLog::all`
    /// itself uses.
    pub grants: Vec<AaGrantDto>,
    pub total_spent: u32,
}

pub fn aa_log(ing: &Ingest) -> AaLogDto {
    let grants = ing
        .aa
        .all()
        .map(|(ts, g)| {
            let catalog = crate::aadata::aa_by_name(&g.name);
            let relevant_stats = catalog
                .and_then(|a| a.description.as_deref())
                .map(|d| {
                    crate::aadata::relevant_stats(d)
                        .into_iter()
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            let cost_modifiers = catalog
                .map(crate::aadata::cost_modifiers)
                .unwrap_or_default();
            AaGrantDto {
                ts_ms: *ts,
                name: g.name.clone(),
                rank: g.rank,
                cost: g.cost,
                category: catalog.map(|a| a.category.clone()),
                description: catalog.and_then(|a| a.description.clone()),
                max_rank: catalog.map(|a| a.ranks),
                cost_progression: catalog.map(|a| a.cost_raw.clone()),
                catalog_certain: catalog.map(|a| a.certain),
                relevant_stats,
                cost_modifiers,
            }
        })
        .collect();
    AaLogDto {
        grants,
        total_spent: ing.aa.total_spent(),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SpellbookEntryDto {
    pub name: String,
    /// "known" (a scribe or memorize *finished* at least once -- see
    /// `ingest::SpellLog`'s own doc) or "possible" (only a "Beginning
    /// to..." line was ever seen, no confirmed finish).
    pub confidence: String,
    /// For "known": when it was confirmed. For "possible": when the
    /// unconfirmed attempt began.
    pub first_seen_ms: Millis,
    /// Everything below is catalog enrichment from `spelldata::Spell` --
    /// all blank/empty together for a name the catalog doesn't carry.
    pub description: Option<String>,
    pub mana: Option<f64>,
    pub casting_time: Option<f64>,
    pub recast_time: Option<f64>,
    pub duration: Option<String>,
    pub target_type: Option<String>,
    pub resist: Option<String>,
    #[serde(default)]
    pub classes: Vec<crate::spelldata::SpellClass>,
    pub icon: Option<String>,
}

fn spellbook_entry(name: &str, confidence: &str, ts: Millis) -> SpellbookEntryDto {
    let catalog = crate::spelldata::spell_by_name(name);
    SpellbookEntryDto {
        name: name.to_string(),
        confidence: confidence.to_string(),
        first_seen_ms: ts,
        description: catalog.and_then(|s| s.description.clone()),
        mana: catalog.and_then(|s| s.mana),
        casting_time: catalog.and_then(|s| s.casting_time),
        recast_time: catalog.and_then(|s| s.recast_time),
        duration: catalog.and_then(|s| s.duration.clone()),
        target_type: catalog.and_then(|s| s.target_type.clone()),
        resist: catalog.and_then(|s| s.resist.clone()),
        classes: catalog.map(|s| s.classes.clone()).unwrap_or_default(),
        icon: catalog.and_then(|s| s.icon.clone()),
    }
}

/// Every spell with at least Possible-tier evidence this session (see
/// `ingest::SpellLog`'s own doc for the Known/Possible distinction),
/// Known ones first, newest-confirmed/newest-attempted first within each
/// tier, each enriched with its own catalog stats -- mana cost, cast/
/// recast time, resist type, and (via `description`) the damage/heal/
/// effect text the wiki itself carries.
pub fn spellbook(ing: &Ingest) -> Vec<SpellbookEntryDto> {
    let mut known: Vec<SpellbookEntryDto> = ing
        .spellbook
        .known()
        .map(|(name, ts)| spellbook_entry(name, "known", ts))
        .collect();
    known.sort_by(|a, b| b.first_seen_ms.cmp(&a.first_seen_ms));

    let mut possible: Vec<SpellbookEntryDto> = ing
        .spellbook
        .possible()
        .map(|(name, ts)| spellbook_entry(name, "possible", ts))
        .collect();
    possible.sort_by(|a, b| b.first_seen_ms.cmp(&a.first_seen_ms));

    known.extend(possible);
    known
}

/// Highest live rank observed cast this session for "You", by catalog
/// base spell name (`Ice Comet` -> `10` for a confirmed "Ice Comet X"
/// cast) -- see `ingest::SpellRanks`' own doc. A spell never cast this
/// session simply has no entry, not a `0` -- there's no such thing as a
/// confirmed rank 0.
pub fn spell_ranks(ing: &Ingest) -> HashMap<String, u8> {
    ing.spell_ranks
        .all()
        .map(|(name, rank)| (name.to_string(), rank))
        .collect()
}
