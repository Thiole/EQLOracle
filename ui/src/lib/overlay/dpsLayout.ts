// why: one source of truth for the DPS meter's layout preset -- the
// backend's own DpsLayout decides what the payload holds, this decides
// the window it has to fit in. commands.rs's dps_meter_dims() mirrors
// these (w, h) pairs for the window's INITIAL open size; this table is
// what a live change resizes to. Same hand-kept contract as ccSize.ts.
export type DpsLayout = 'minimal' | 'condensed' | 'full';

export const DEFAULT_DPS_LAYOUT: DpsLayout = 'condensed';

const LAYOUTS: readonly DpsLayout[] = ['minimal', 'condensed', 'full'];

/** why: an unrecognized value falls back to the default rather than
 * erroring -- same contract as asCcSize's own doc. */
export function asDpsLayout(v: string | null | undefined): DpsLayout {
  return (LAYOUTS as readonly string[]).includes(v ?? '') ? (v as DpsLayout) : DEFAULT_DPS_LAYOUT;
}

/** why: minimal drops the enemy side entirely, so it needs the least
 * room; full gives every enemy its own row and needs the most. */
export const DPS_LAYOUT_WINDOW_DIMS: Record<DpsLayout, { w: number; h: number }> = {
  minimal: { w: 360, h: 150 },
  condensed: { w: 360, h: 240 },
  full: { w: 360, h: 330 },
};
