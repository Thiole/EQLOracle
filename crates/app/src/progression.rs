//! why: AA and Spellbook progression, from `ingest::AaLog`/`SpellLog`,
//! enriched (best-effort) with the scraped catalogs. Both "at a cost of"
//! log forms exhaustively covered (101/101 in the reference log), plus
//! scribing (596/593) and memorize signals -- see `ingest::SpellLog`.

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
    /// why: catalog enrichment below, all None together if unmatched --
    /// never a reason to drop the real grant itself
    pub category: Option<String>,
    pub description: Option<String>,
    /// why: total ranks, so a view can show "rank 2 of 4"
    pub max_rank: Option<u32>,
    /// why: wiki's raw per-rank cost string, shown as-is
    pub cost_progression: Option<String>,
    /// why: whether the scrape is confident this entry's numbers are complete
    pub catalog_certain: Option<bool>,
    /// why: stat rows this AA's description suggests it affects, best-effort
    pub relevant_stats: Vec<String>,
    /// why: mana/cast-time effects at every rank; empty for most AAs
    pub cost_modifiers: Vec<crate::aadata::CostModifier>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AaLogDto {
    /// why: log-time order, oldest first, same convention `AaLog::all` uses
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
    /// why: "known" (finished at least once) or "possible" (unconfirmed attempt)
    pub confidence: String,
    /// why: confirmed time for known, attempt-began time for possible
    pub first_seen_ms: Millis,
    /// why: catalog enrichment below, all blank together if unmatched
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

/// why: every spell with Possible+ evidence this session, Known first,
/// newest first within each tier, enriched with catalog stats
pub fn spellbook(ing: &Ingest) -> Vec<SpellbookEntryDto> {
    let mut known: Vec<SpellbookEntryDto> = ing
        .spellbook
        .known()
        .map(|(name, ts)| spellbook_entry(name, "known", ts))
        .collect();
    known.sort_by_key(|b| std::cmp::Reverse(b.first_seen_ms));

    let mut possible: Vec<SpellbookEntryDto> = ing
        .spellbook
        .possible()
        .map(|(name, ts)| spellbook_entry(name, "possible", ts))
        .collect();
    possible.sort_by_key(|b| std::cmp::Reverse(b.first_seen_ms));

    known.extend(possible);
    known
}

/// why: highest observed cast rank by base spell name; no entry if never cast
pub fn spell_ranks(ing: &Ingest) -> HashMap<String, u8> {
    ing.spell_ranks
        .all()
        .map(|(name, rank)| (name.to_string(), rank))
        .collect()
}
