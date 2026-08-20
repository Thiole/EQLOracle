// Ported directly from the legacy app (ui/app-legacy/app.js) -- same
// output, same edge cases, not reinvented.

/** "3:07" -- minutes:seconds, zero-padded. */
export function fmtDuration(ms: number | null | undefined): string {
  const total = Math.max(0, Math.round((ms ?? 0) / 1000));
  const m = Math.floor(total / 60);
  const s = total % 60;
  return `${m}:${String(s).padStart(2, '0')}`;
}

/** "3m 7s" (or just "7s" under a minute) -- for a standalone duration, not a clock. */
export function fmtTtk(ms: number): string {
  const total = Math.max(0, Math.round(ms / 1000));
  const m = Math.floor(total / 60);
  const s = total % 60;
  return m > 0 ? `${m}m ${s}s` : `${s}s`;
}
