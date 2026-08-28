// why: one source of truth for the CC Tracker widget's "layout" (really
// size) preset -- CCStatusWidget's own square/text classes and
// OverlayApp's own window-resize dims both read this table, so they can
// never drift out of sync with each other. commands.rs's own
// cc_tracker_dims() mirrors these same (w, h) pairs on the Rust side for
// the window's INITIAL open size (read from the persisted preference
// before the frontend's even loaded) -- this table is what a live size
// CHANGE (already-open window) resizes to instead.
export type CcSize = 'small' | 'medium' | 'large';

export const DEFAULT_CC_SIZE: CcSize = 'small';

const SIZES: readonly CcSize[] = ['small', 'medium', 'large'];

/** why: an unrecognized value (an old/downgraded install, a hand-edited
 * prefs file) falls back to the default rather than erroring -- same
 * "unknown slug just falls back" contract as preferences.rs's own
 * `theme` field doc. */
export function asCcSize(v: string | null | undefined): CcSize {
  return (SIZES as readonly string[]).includes(v ?? '') ? (v as CcSize) : DEFAULT_CC_SIZE;
}

/** why: per-size Tailwind classes for one square -- height and label
 * text size scale together, the gap between squares scales too so
 * bigger squares don't end up crammed against each other. */
export const CC_SIZE_CLASSES: Record<CcSize, { square: string; gap: string }> = {
  small: { square: 'h-5 text-[10px]', gap: 'gap-1' },
  medium: { square: 'h-7 text-[12px]', gap: 'gap-1.5' },
  large: { square: 'h-9 text-[14px]', gap: 'gap-2' },
};

/** why: the overlay window's own logical-pixel size at each preset --
 * see this file's own top doc. Just big enough for 3 squares at that
 * size plus the shared panel chrome (CCTrackerWidget's own p-2,
 * OverlayApp's own p-2 wrapper), no leftover dead space below the
 * squares the way one fixed window size would leave for "small". */
export const CC_SIZE_WINDOW_DIMS: Record<CcSize, { w: number; h: number }> = {
  small: { w: 220, h: 48 },
  medium: { w: 250, h: 60 },
  large: { w: 280, h: 76 },
};
