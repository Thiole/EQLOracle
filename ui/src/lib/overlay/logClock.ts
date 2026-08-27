// why: real bug -- Skill Tracker's since_ms/ready_at_ms come from the
// backend's log timestamps, which crates/core/src/header.rs parses as
// "naive civil time treated as UTC" (fixed-offset, no chrono -- every
// backend-internal comparison is relative between two log timestamps,
// never needs the real timezone). Comparing that directly against
// Date.now() (a real UTC epoch) introduces a fixed skew equal to the
// machine's UTC offset -- a fresh landing in EDT (UTC-4) always showed
// as an already-expired 0:00 timer, ready_at_ms permanently ~4 hours
// "behind" Date.now().
//
// Fix lives here, not in the backend: reconstruct the SAME naive
// representation from the browser's own clock, so both sides agree.
// new Date()'s own LOCAL calendar fields (getFullYear/getMonth/...,
// never the getUTC* variants) are the real wall-clock numbers a human
// -- and the game log sitting right next to this overlay, on the same
// machine -- would show; feeding those into Date.UTC() produces the
// same "civil time read as if it were UTC" number the backend computes
// from the log's own bracketed timestamp. The overlay and the log it's
// reading are always on the same machine, so this always matches.
export function logClockNowMs(): number {
  const d = new Date();
  return Date.UTC(
    d.getFullYear(),
    d.getMonth(),
    d.getDate(),
    d.getHours(),
    d.getMinutes(),
    d.getSeconds(),
    d.getMilliseconds(),
  );
}
