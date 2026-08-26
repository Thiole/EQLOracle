// why: pure formatter, no Svelte/DOM -- Combat.svelte's copy button calls
// this, but it's independently testable and not tied to one caller.
// Shaped for pasting into EQ's own chat window: one line, no monospace/
// tabs (proportional font, doesn't exist there), and a newline in a paste
// fires as its own separate chat message -- several lines means several
// spammed messages, so this stays one line no matter the raid size.

import type { AllyDto, CombatSummaryDto } from '$lib/tauri/api';
import { fmtDuration } from '$lib/format';

export interface ReportHeader {
  /** null for an aggregate selection spanning several fights. */
  target: string | null;
  tag: 'kill' | 'wipe' | 'reset' | 'ongoing' | null;
  fightCount: number;
}

/** why: matches "the pasted mock and any real block of numbers" the
 * request pushed back on -- compact so a raid-sized ally list still fits
 * one line, e.g. "115.2k" not "115,200" */
function fmtCompact(n: number): string {
  if (n < 1000) return n.toFixed(n < 10 ? 1 : 0);
  return `${(n / 1000).toFixed(1)}k`;
}

/** why: capped -- a full raid's worth of allies on one line stops being
 * readable long before it stops fitting in EQ's per-line length limit */
const MAX_ALLIES = 8;

export function buildCombatReport(header: ReportHeader, summary: CombatSummaryDto, allies: AllyDto[]): string {
  const title =
    header.target === null
      ? `Aggregate (${header.fightCount} fight${header.fightCount === 1 ? '' : 's'}, ${fmtDuration(summary.duration_ms)})`
      : `${header.target} (${header.tag ?? '?'}, ${fmtDuration(summary.duration_ms)})`;

  const teamLine = `Team ${fmtCompact(summary.total_damage)} dmg, ${fmtCompact(summary.dps)} dps`;
  const incoming = summary.enemy_damage > 0 ? `, incoming ${fmtCompact(summary.enemy_damage)} dmg` : '';

  // why: just total + dps per ally -- name/crit/hit/resist per entry read
  // as a wall of numbers on one line, the two that matter for "who did
  // what" are total and dps
  const shown = allies.slice(0, MAX_ALLIES).map((a) => `${a.name} ${fmtCompact(a.total)}/${fmtCompact(a.dps)}dps`);
  const rest = allies.length - shown.length;
  const allyLine = shown.length ? ` -- ${shown.join(', ')}${rest > 0 ? `, +${rest} more` : ''}` : '';

  return `${title} -- ${teamLine}${incoming}${allyLine}`;
}
