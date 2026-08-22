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
