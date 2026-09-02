import { type ClassValue, clsx } from 'clsx';
import { twMerge } from 'tailwind-merge';

// The standard shadcn-svelte helper -- merges conditional class lists
// (clsx) and resolves conflicting Tailwind utilities in favor of the
// last one given (tailwind-merge), so a component's own default classes
// can be safely overridden by whatever a caller passes in via `class`.
export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

// Standard shadcn-svelte helper type: a component's own props, plus an
// optional bindable `ref` to the underlying DOM element it renders.
// Every generated ui/ component imports this.
export type WithElementRef<T, E extends HTMLElement = HTMLElement> = T & { ref?: E | null };

// bits-ui wrapper components (select, scroll-area, ...) re-derive their
// own props from the underlying primitive's props type, minus the
// `child`/`children` snippet props they replace with their own -- these
// three are the standard shadcn-svelte helpers for that.
export type WithoutChild<T> = T extends { child?: any } ? Omit<T, 'child'> : T;
export type WithoutChildren<T> = T extends { children?: any } ? Omit<T, 'children'> : T;
export type WithoutChildrenOrChild<T> = WithoutChildren<WithoutChild<T>>;

/** why: the scrape disambiguates a real zone-name collision ("Cazic
 * Thule" the playable zone vs. an unrelated wiki deity/lore page also
 * titled "Cazic Thule") by appending " (Zone)" to that one zone's own
 * canonical `Zone::name` -- meaningful for exact-name matching
 * everywhere in the backend, meaningless (and confusing) to show a
 * player. Strips *only* that exact literal suffix, never any other
 * parenthetical -- `Chardok (Post-Revamp)` / `Chardok (Pre-Revamp)`
 * carry real information (two genuinely different zone versions) and
 * must never be silently hidden the same way. Display-only: never call
 * this on a name before sending it back to a Tauri command, which needs
 * the real canonical string. */
export function displayZoneName(name: string): string {
  return name.endsWith(' (Zone)') ? name.slice(0, -' (Zone)'.length) : name;
}

/** why: the backend stamps every log line as its wall-clock time read as
 * UTC (core/header.rs: days_from_civil + seconds, no zone), so a log
 * millisecond must be shown with the UTC getters -- the local ones shift
 * it by the machine's offset (4 hours off in practice). Same rule in
 * reverse for anything the user types as a wall-clock time. */
export function fmtLogTime(ms: number): string {
  const d = new Date(ms);
  const p = (n: number) => String(n).padStart(2, '0');
  return `${p(d.getUTCHours())}:${p(d.getUTCMinutes())}:${p(d.getUTCSeconds())}`;
}
export function fmtLogDate(ms: number): string {
  const d = new Date(ms);
  const p = (n: number) => String(n).padStart(2, '0');
  return `${d.getUTCFullYear()}-${p(d.getUTCMonth() + 1)}-${p(d.getUTCDate())}`;
}
/** why: log ms -> the value a datetime-local input wants (wall clock) */
export function logMsToLocalInput(ms: number): string {
  const d = new Date(ms);
  const p = (n: number) => String(n).padStart(2, '0');
  return `${d.getUTCFullYear()}-${p(d.getUTCMonth() + 1)}-${p(d.getUTCDate())}T${p(d.getUTCHours())}:${p(d.getUTCMinutes())}`;
}
/** why: datetime-local value (wall clock) -> log ms; null when unparsable */
export function localInputToLogMs(v: string): number | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})(?::(\d{2}))?$/.exec(v);
  if (!m) return null;
  return Date.UTC(+m[1], +m[2] - 1, +m[3], +m[4], +m[5], m[6] ? +m[6] : 0);
}
