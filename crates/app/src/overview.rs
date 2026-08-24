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
    /// why: None only before a single line has been parsed at all
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

pub fn session(ing: &Ingest) -> SessionDto {
    let now = ing.now_ms();
    let session_start_ms = ing.session_start();
    let session_duration_ms = session_start_ms.map(|s| now.saturating_sub(s)).unwrap_or(0);

    let (platinum_per_hour, xp_pct_per_hour) = if session_duration_ms >= MIN_SESSION_MS_FOR_RATE {
        let start = session_start_ms.expect("duration is only nonzero once a session has started");
        let hours = session_duration_ms as f64 / 3_600_000.0;
        let copper = sum_since(ing, EventKind::Currency, start);
        let milli_pct = sum_since(ing, EventKind::Xp, start);
        (
            Some((copper as f64 / 1000.0) / hours),
            Some((milli_pct as f64 / 1000.0) / hours),
        )
    } else {
        (None, None)
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

    SessionDto {
        afk: ing.currently_afk(),
        session_start_ms,
        session_duration_ms,
        platinum_per_hour,
        xp_pct_per_hour,
        current_level,
        progress_pct,
        eta_hours,
    }
}
