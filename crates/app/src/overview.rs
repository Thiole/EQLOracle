//! Overview module: session-scoped rate stats -- platinum/hour, xp%/hour,
//! and an estimated time to the next level -- averaged over "this
//! session" as `Ingest::session_start` defines it (see that method's own
//! doc): however long it's been since the player last stopped being AFK,
//! or since the log itself started if they never have. Averaging over the
//! whole file's span instead would silently flatten hours of AFK downtime
//! into the rate, which is exactly the failure mode `session_start`
//! exists to avoid.
//!
//! Both rates are suffix scans over `Store`, not full scans: `Store::ts`
//! is monotonically non-decreasing (rows are appended in log order,
//! whether from a live tail or a chronological backfill), so `partition_
//! point` finds the first row at or after a given timestamp in O(log n),
//! and everything from there to the end is exactly "what's happened since
//! that instant" -- no more, no less. This is the same discipline
//! `monsters::xp_by_mob`'s own doc requires of itself: this module runs on
//! a UI poll, so an O(store length) full scan here would be the same
//! mistake that had to be fixed there, just in a new place.

use crate::ingest::Ingest;
use eqlp_source::Millis;
use eqlp_store::EventKind;
use serde::Serialize;

/// Below this, a rate is reported as unavailable rather than computed --
/// dividing a real total by a handful of seconds produces a number that's
/// technically an average but reads as a wild, meaningless spike (a
/// single lucky corpse two seconds after returning from AFK does not mean
/// "40,000 platinum/hour"). Long enough to smooth past one lucky event,
/// short enough that a genuine short session still gets a real number
/// well before it ends.
const MIN_SESSION_MS_FOR_RATE: Millis = 60_000;

#[derive(Debug, Clone, Serialize)]
pub struct SessionDto {
    /// Whether AFK as of the most recently parsed line.
    pub afk: bool,
    /// When this session (see this module's doc) began -- `None` only
    /// before a single line has been parsed at all.
    pub session_start_ms: Option<Millis>,
    pub session_duration_ms: Millis,
    /// `None` below `MIN_SESSION_MS_FOR_RATE` -- see that constant's doc.
    pub platinum_per_hour: Option<f64>,
    pub xp_pct_per_hour: Option<f64>,
    /// `Ingest::levels`' own most-recently-observed level -- `None` under
    /// the same condition that method is: no `level.up` line anywhere yet
    /// in this file's history (see its own doc for why that mostly means
    /// "you've been this level the whole time", not "level unknown").
    pub current_level: Option<u8>,
    /// % already banked toward the *next* level -- summed `Xp` gains
    /// since the last `level.up` line, which is deliberately not the same
    /// window as the session: a level doesn't reset just because the
    /// player went AFK partway through it, only a ding does. `None` if
    /// there's no `level.up` baseline yet to measure progress against.
    pub progress_pct: Option<f64>,
    /// `(100.0 - progress_pct) / xp_pct_per_hour` -- `None` whenever
    /// either half is unavailable (no level baseline, or no session rate
    /// yet to extrapolate from), or whenever the rate is `0.0` (no gain
    /// at all this session -- extrapolating a "hours to level" from zero
    /// progress per hour is an infinity, not a real estimate).
    pub eta_hours: Option<f64>,
}

/// Sum of `Store::amount` for every `kind`-matching row at or after
/// `start_ts` (inclusive), via `partition_point` rather than a full scan
/// -- see this module's own doc.
fn sum_since(ing: &Ingest, kind: EventKind, start_ts: Millis) -> u64 {
    sum_from_index(ing, kind, ing.store.ts.partition_point(|&t| t < start_ts))
}

/// Same as `sum_since`, but strictly *after* `ts` -- what `progress_pct`
/// needs: the XP gain that itself completed the last level belongs to
/// that level's own total, not to progress toward the next one, and at
/// the log's one-second resolution it can genuinely share a timestamp
/// with the `level.up` line it caused. `sum_since`'s inclusive boundary
/// would double-count that gain into both; this excludes it.
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
