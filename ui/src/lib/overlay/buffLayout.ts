// why: one source of truth for the Group Buffs widget's layout preset --
// GroupBuffsWidget's own rendering and OverlayApp's window-resize dims
// both read this, so they cannot drift. commands.rs's own
// group_buffs_dims() mirrors the same (w, h) pairs for the window's
// INITIAL open size, read from the persisted preference before the
// frontend has loaded; this table is what a live change resizes to.
//
// The layout changes NOTHING in the app -- Spencer: "it doesnt change any
// information in app. just minimizes the screen space in game".
export type BuffLayout = 'full' | 'minimal';

export const DEFAULT_BUFF_LAYOUT: BuffLayout = 'full';

const LAYOUTS: readonly BuffLayout[] = ['full', 'minimal'];

/** why: an unrecognized value (an old install, a hand-edited prefs file)
 * falls back to the default rather than erroring -- same contract as
 * asCcSize's own doc. */
export function asBuffLayout(v: string | null | undefined): BuffLayout {
  return (LAYOUTS as readonly string[]).includes(v ?? '') ? (v as BuffLayout) : DEFAULT_BUFF_LAYOUT;
}

/** why: the overlay window's own logical-pixel size per layout. Minimal
 * is one verdict line and nothing else, so the window shrinks to match --
 * a one-line widget inside a 280x220 window would not minimize anything. */
export const BUFF_LAYOUT_WINDOW_DIMS: Record<BuffLayout, { w: number; h: number }> = {
  full: { w: 280, h: 220 },
  minimal: { w: 180, h: 46 },
};
