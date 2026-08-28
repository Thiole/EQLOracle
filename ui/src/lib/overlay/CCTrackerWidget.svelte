<script lang="ts">
  // why: CC status (Root/Stun/Fear) as its own peer overlay widget --
  // same footing as DpsMeterWidget/SkillTrackerWidget/DropWatchWidget
  // (own on/off, own opacity, own real OS window), not a section stacked
  // inside a bigger panel. This file owns the shared panel chrome
  // (background/opacity/text-shadow, same recipe every overlay widget
  // here uses); CCStatusWidget.svelte stays the pure, dumb "turn a
  // {label, active}[] list into squares" component, reused as-is.
  //
  // Deliberately tiny -- three squares don't need a resizable data
  // table's worth of window; OverlayApp.svelte resizes the real OS
  // window to match `size` (see ccSize.ts's own doc) whenever it
  // changes, this component just forwards it to CCStatusWidget.
  import type { StatusEffectsDto } from '$lib/tauri/api';
  import CCStatusWidget from './CCStatusWidget.svelte';
  import type { CcSize } from './ccSize';

  let {
    status,
    opacity,
    overallOpacity,
    size = 'small',
  }: {
    status: StatusEffectsDto | null;
    opacity: number;
    // why: the SEPARATE "everything" fade -- see DpsMeterWidget's own doc
    overallOpacity: number;
    size?: CcSize;
  } = $props();
</script>

<div
  class="rounded-md p-2 text-[12px] font-semibold"
  style:background-color="color-mix(in srgb, var(--background) {opacity * 100}%, transparent)"
  style:opacity={overallOpacity}
  style:text-shadow="0 1px 2px rgba(0, 0, 0, 0.9), 0 0px 4px rgba(0, 0, 0, 0.6)"
>
  <CCStatusWidget {status} {size} />
</div>
