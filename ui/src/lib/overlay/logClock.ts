// why: real bug, caught live -- Skill Tracker's own since_ms/ready_at_ms
// (cooldowns and target effects alike) come from the backend's own log
// timestamps, which crates/core/src/header.rs deliberately parses as
// "naive civil time treated as UTC" ("fixed-offset, no chrono" -- every
// backend-internal comparison is relative between two log timestamps,
// so it never needs to know the real timezone at all). Comparing that
// value directly against Date.now() (a REAL UTC epoch) introduces a
// fixed skew equal to the machine's own UTC offset -- Spencer's own
// report: a fresh Tashania landing in EDT (UTC-4) always showed as an
// already-expired red 0:00 timer, because ready_at_ms was permanently
// ~4 hours "behind" Date.now(), never anywhere close to catching up.
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
