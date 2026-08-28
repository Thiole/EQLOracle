//! why: session-scoped rate stats -- plat/hour, xp%/hour, ETA to next level
//!
//! Averaged over `Ingest::session_start`, not the whole file -- avoids
//! flattening AFK downtime into the rate. Suffix scans via `partition_point`
//! (O(log n)), not full scans -- this runs on a UI poll.

use crate::ingest::Ingest;
use eqlp_source::Millis;
use eqlp_store::EventKind;
use serde::Serialize;

/// why: below this, report unavailable -- a short window spikes wildly
const MIN_SESSION_MS_FOR_RATE: Millis = 60_000;

#[derive(Debug, Clone, Serialize)]
pub struct SessionDto {
    /// Whether AFK as of the most recently parsed line.
    pub afk: bool,
    /// why: None only before a single line has been parsed at all.
    /// Reflects `Ingest::session_start` -- AFK-return or a manual
    /// "restart" (`reset_session`), whichever is later.
    pub session_start_ms: Option<Millis>,
    pub session_duration_ms: Millis,
    /// why: None below `MIN_SESSION_MS_FOR_RATE`
    pub platinum_per_hour: Option<f64>,
    pub xp_pct_per_hour: Option<f64>,
    /// why: None means no `level.up` line yet, not "level unknown"
    pub current_level: Option<u8>,
    /// why: summed Xp since last level.up -- doesn't reset on AFK, only on ding
    pub progress_pct: Option<f64>,
    /// why: None if either half unavailable, or rate is 0 (would be infinity)
    pub eta_hours: Option<f64>,
    /// why: real Loot rows, every "Mote of <tier> Potential" tier summed
    /// together -- see `sum_motes_since`'s own doc for why a combined
    /// total, not a per-tier breakdown
    pub motes_found: u64,
    /// why: None below `MIN_SESSION_MS_FOR_RATE`, same gate as the other rates
    pub motes_per_hour: Option<f64>,
    /// why: current_level minus the level as of session start -- None
    /// when the level at session start itself isn't known (no level.up
    /// line has ever landed by then), never guessed as 0
    pub levels_gained: Option<u8>,
    /// why: real AA cost sum since session start, from the same
    /// timestamped grants `progression::aa_log` already exposes --
    /// "how many points you've spent this session", not a rate (AA
    /// grants are too bursty/rare for a per-hour number to mean much)
    pub aa_spent: u32,
    /// why: per-tier companion to `motes_found`'s combined total -- only
    /// tiers actually seen this session, ascending by `tier`. See
    /// `MOTE_TIER_ORDER`'s own doc for where the tier numbers come from.
    pub mote_tiers: Vec<MoteTierDto>,
}

/// why: all 9 names verified real against the wiki scrape's own
/// "Mote of <tier> Potential" component stubs (spelling and all); 7 of
/// them (all but Ascendant/Infinite) also confirmed as real *loot*
/// lines in both reference logs -- the other 2 only turned up in chat
/// text so far (players mentioning them as rare), never a "You looted"
/// line, evidently just rare drops rather than unreal names.
///
/// The ORDER, though, is not wiki-confirmed: the scrape has no full
/// Item record for Motes at all (no stats/icon, just a bare
/// crafting-component stub with no tier field), so ascending
/// low-to-high here is inferred purely from the English magnitude
/// gradient in the names themselves, not a real in-game numbering --
/// hence the UI shows the tier *name*, never a fabricated "Tier N".
const MOTE_TIER_ORDER: [&str; 9] = [
    "Infinitesimal",
    "Minor",
    "Lesser",
    "Greater",
    "Major",
    "Superior",
    "Grand",
    "Ascendant",
    "Infinite",
];

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MoteTierDto {
    /// why: 1-based, ascending -- see `MOTE_TIER_ORDER`'s own doc
    pub tier: u8,
    pub name: String,
    pub count: u64,
}

/// why: sum matching rows at/after `start_ts`, via `partition_point`
fn sum_since(ing: &Ingest, kind: EventKind, start_ts: Millis) -> u64 {
    sum_from_index(ing, kind, ing.store.ts.partition_point(|&t| t < start_ts))
}

/// why: strictly after `ts` -- excludes the gain that completed the ding
/// itself, which can share a timestamp with the level.up line
fn sum_after(ing: &Ingest, kind: EventKind, ts: Millis) -> u64 {
    sum_from_index(ing, kind, ing.store.ts.partition_point(|&t| t <= ts))
}

fn sum_from_index(ing: &Ingest, kind: EventKind, start_i: usize) -> u64 {
    (start_i..ing.store.len())
        .filter(|&j| ing.store.kind[j] == kind)
        .map(|j| ing.store.amount[j])
        .sum()
}

/// why: every "Mote of <tier> Potential" (Infinitesimal/Minor/Lesser/.../
/// Ascendant, confirmed 9 real tiers in the reference log) summed as one
/// combined total, not broken out per tier -- the Session card's own ask
/// was "motes found", a single number to watch climb while farming, not
/// a tier-by-tier breakdown (that's a real, easy follow-up if wanted,
/// not assumed here). Loot rows only, matching sum_since's own shape.
fn sum_motes_since(ing: &Ingest, start_ts: Millis) -> u64 {
    let start_i = ing.store.ts.partition_point(|&t| t < start_ts);
    (start_i..ing.store.len())
        .filter(|&j| ing.store.kind[j] == EventKind::Loot)
        .filter(|&j| {
            ing.store
                .ability_name(ing.store.ability[j])
                .starts_with("Mote of ")
        })
        .map(|j| ing.store.amount[j])
        .sum()
}

/// why: per-tier breakdown for the Session card's icon row -- companion
/// to `sum_motes_since`'s combined total, not a replacement (that's
/// still the headline "N motes/hr" number)
fn motes_by_tier_since(ing: &Ingest, start_ts: Millis) -> Vec<MoteTierDto> {
    let start_i = ing.store.ts.partition_point(|&t| t < start_ts);
    let mut counts = [0u64; MOTE_TIER_ORDER.len()];
    for j in start_i..ing.store.len() {
        if ing.store.kind[j] != EventKind::Loot {
            continue;
        }
        let name = ing.store.ability_name(ing.store.ability[j]);
        let Some(rest) = name.strip_prefix("Mote of ") else {
            continue;
        };
        let Some(tier_name) = rest.strip_suffix(" Potential") else {
            continue;
        };
        if let Some(i) = MOTE_TIER_ORDER.iter().position(|&t| t == tier_name) {
            counts[i] += ing.store.amount[j];
        }
    }
    counts
        .iter()
        .enumerate()
        .filter(|&(_, &c)| c > 0)
        .map(|(i, &count)| MoteTierDto {
            tier: i as u8 + 1,
            name: MOTE_TIER_ORDER[i].to_string(),
            count,
        })
        .collect()
}

pub fn session(ing: &Ingest) -> SessionDto {
    let now = ing.now_ms();
    let session_start_ms = ing.session_start();
    let session_duration_ms = session_start_ms.map(|s| now.saturating_sub(s)).unwrap_or(0);

    let motes_found = session_start_ms.map_or(0, |s| sum_motes_since(ing, s));
    let mote_tiers = session_start_ms.map_or(Vec::new(), |s| motes_by_tier_since(ing, s));

    let (platinum_per_hour, xp_pct_per_hour, motes_per_hour) = if session_duration_ms
        >= MIN_SESSION_MS_FOR_RATE
    {
        let start = session_start_ms.expect("duration is only nonzero once a session has started");
        let hours = session_duration_ms as f64 / 3_600_000.0;
        let copper = sum_since(ing, EventKind::Currency, start);
        let milli_pct = sum_since(ing, EventKind::Xp, start);
        (
            Some((copper as f64 / 1000.0) / hours),
            Some((milli_pct as f64 / 1000.0) / hours),
            Some(motes_found as f64 / hours),
        )
    } else {
        (None, None, None)
    };

    let current_level = ing.levels.latest();
    let progress_pct = ing
        .levels
        .latest_ts()
        .map(|ding_ts| sum_after(ing, EventKind::Xp, ding_ts) as f64 / 1000.0);

    let eta_hours = match (progress_pct, xp_pct_per_hour) {
        (Some(progress), Some(rate)) if rate > 0.0 && progress < 100.0 => {
            Some((100.0 - progress) / rate)
        }
        _ => None,
    };

    // why: None (not 0) when the level *at session start* was never
    // itself confirmed by a real level.up line -- an honest "don't know
    // your starting point" beats guessing zero gained
    let levels_gained = match (
        current_level,
        session_start_ms.and_then(|s| ing.levels.at(s)),
    ) {
        (Some(cur), Some(start)) => Some(cur.saturating_sub(start)),
        _ => None,
    };

    let aa_spent = session_start_ms.map_or(0, |start| {
        ing.aa
            .all()
            .filter(|(ts, _)| *ts >= start)
            .map(|(_, g)| g.cost)
            .sum()
    });

    SessionDto {
        afk: ing.currently_afk(),
        session_start_ms,
        session_duration_ms,
        platinum_per_hour,
        xp_pct_per_hour,
        current_level,
        progress_pct,
        eta_hours,
        motes_found,
        motes_per_hour,
        levels_gained,
        aa_spent,
        mote_tiers,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::{backfill_lines, framed_lines};
    use crate::parser::build_engine;

    fn run(log: &str) -> Ingest {
        let engine = build_engine().expect("pack builds");
        let bytes = log.as_bytes();
        let lines = framed_lines(bytes);
        let mut ing = Ingest::default();
        backfill_lines(&mut ing, &engine, &lines, lines.len());
        ing
    }

    /// why: real line -- confirms the combined-tier sum, not just one tier
    #[test]
    fn motes_are_summed_across_every_tier() {
        let ing = run(
            "[Tue Jul 28 15:31:55 2026] You looted a Mote of Infinitesimal Potential from a dune spiderling's corpse and stored it in your currency\r\n\
             [Tue Jul 28 19:41:42 2026] You looted a Mote of Minor Potential from a gnoll's corpse and stored it in your currency\r\n",
        );
        assert_eq!(session(&ing).motes_found, 2);
    }

    /// why: real bug shape -- combined total and per-tier breakdown must
    /// agree, and tiers must come out ordered ascending, only-seen-ones
    #[test]
    fn mote_tiers_break_down_the_combined_total_by_tier() {
        let ing = run(
            "[Tue Jul 28 15:31:55 2026] You looted a Mote of Minor Potential from a gnoll's corpse and stored it in your currency\r\n\
             [Tue Jul 28 15:32:00 2026] You looted a Mote of Infinitesimal Potential from a dune spiderling's corpse and stored it in your currency\r\n\
             [Tue Jul 28 15:32:05 2026] You looted a Mote of Infinitesimal Potential from a dune spiderling's corpse and stored it in your currency\r\n",
        );
        let s = session(&ing);
        assert_eq!(s.motes_found, 3);
        assert_eq!(
            s.mote_tiers,
            vec![
                MoteTierDto {
                    tier: 1,
                    name: "Infinitesimal".into(),
                    count: 2
                },
                MoteTierDto {
                    tier: 2,
                    name: "Minor".into(),
                    count: 1
                },
            ],
            "ascending by tier, not by first-seen order"
        );
    }

    /// why: honest unknown, not a guessed 0 -- no level.up line at all
    /// means the level as of session start was never actually confirmed
    #[test]
    fn levels_gained_is_none_with_no_level_up_line_at_all() {
        let ing = run("[Tue Jul 28 15:31:55 2026] You looted a Mote of Minor Potential from a gnoll's corpse and stored it in your currency\r\n");
        assert_eq!(session(&ing).levels_gained, None);
    }

    /// why: real bug shape this guards against -- a ding *before*
    /// session_start (here, before an AFK-return moves session_start
    /// forward) must count as "already at this level coming in", not
    /// inflate levels_gained
    #[test]
    fn levels_gained_only_counts_dings_after_session_start() {
        let ing = run(
            "[Tue Jul 28 15:00:00 2026] You have gained a level! Welcome to level 5!\r\n\
             [Tue Jul 28 15:00:01 2026] You are now A.F.K. (Away From Keyboard).\r\n\
             [Tue Jul 28 15:00:02 2026] You are no longer A.F.K. (Away From Keyboard).\r\n\
             [Tue Jul 28 15:31:55 2026] You have gained a level! Welcome to level 6!\r\n",
        );
        let s = session(&ing);
        assert_eq!(s.current_level, Some(6));
        assert_eq!(
            s.levels_gained,
            Some(1),
            "session_start moved to the afk-off (15:00:02), after the level-5 ding and \
             before the level-6 one -- level 5 was already the level coming into the \
             session, only the level-6 ding happened inside it"
        );
    }

    /// why: real bug this whole feature is for -- reset_session must
    /// zero out levels_gained/motes_found for anything that already
    /// happened before the click, and pick back up cleanly after
    #[test]
    fn reset_session_starts_every_session_stat_over_from_zero() {
        let mut ing = run("[Tue Jul 28 15:00:00 2026] You looted a Mote of Minor Potential from a gnoll's corpse and stored it in your currency\r\n[Tue Jul 28 15:00:01 2026] You have gained a level! Welcome to level 5!\r\n");
        assert_eq!(session(&ing).motes_found, 1);

        ing.reset_session();
        let after_reset = session(&ing);
        assert_eq!(
            after_reset.motes_found, 0,
            "the earlier mote is before the reset"
        );
        assert_eq!(
            after_reset.levels_gained,
            Some(0),
            "level 5 is now the level as of the (later) session start"
        );
    }

    /// why: real AA line, confirms the sum uses cost not a flat per-grant count
    #[test]
    fn aa_spent_sums_real_cost_not_grant_count() {
        let ing = run(
            "[Fri Jul 31 16:55:33 2026] You have gained the ability \"Spell Casting Deftness\" at a cost of 2 ability points.\r\n\
             [Mon Aug 10 09:00:00 2026] You have improved Spell Casting Deftness 2 at a cost of 4 ability points.\r\n",
        );
        assert_eq!(session(&ing).aa_spent, 6);
    }
}
