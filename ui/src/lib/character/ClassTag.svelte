<script lang="ts">
  // why: 3-letter class code, colored by archetype group (not per-class rainbow)
  import { CLASS_ARCHETYPE, ARCHETYPE_COLOR } from './constants';

  let { code, muted = false }: { code: string; muted?: boolean } = $props();
  const color = $derived(ARCHETYPE_COLOR[CLASS_ARCHETYPE[code] ?? ''] ?? '#8b8f97');
</script>

<span
  class="inline-flex shrink-0 items-center rounded-sm px-1 py-px text-[9px] font-bold tracking-wide {muted ? 'opacity-60' : ''}"
  style="color: {color}; background: color-mix(in srgb, {color} 18%, transparent); border: 1px solid color-mix(in srgb, {color} 45%, transparent);"
>
  {code}
</span>
