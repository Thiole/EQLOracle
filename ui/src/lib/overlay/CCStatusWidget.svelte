<script lang="ts">
  // why: compact CC (crowd control) status row -- Root/Stun/Fear as
  // small side-by-side squares, dynamic solid-fill coloring on/off (same
  // convention SkyQuests' own tracking bell uses: a solid bad-toned fill
  // when active, a dim outline when not -- not a separate dot+label
  // pair, the square itself *is* the label). Pure and dumb on purpose --
  // knows nothing about panels/opacity/windows, just turns a
  // {label, active}[] list into squares, so a 4th CC type (Snare-as-
  // distinct-from-Root, Mesmerize, Silence, a real "casting" indicator,
  // ...) later is one more list entry, not new markup. `size` is the
  // one layout knob (see ccSize.ts's own doc) -- small/medium/large,
  // 'small' if unset. Rendered inside
  // CCTrackerWidget.svelte, its own standalone overlay widget/window --
  // same footing as DPS meter/Skill Tracker/Drop Watch, not a section of
  // any of them.
  //
  // Backed by real, per-effect ON/OFF log lines (effects.rs's own doc) --
  // Stun and Root are clean wiki/log-confirmed pairs; Fear is a rough,
  // curated set of real spell text (not exhaustive -- see
  // state.you_feared's own doc in packs/eql.toml), sharing one real
  // wear-off line across every source. All three reuse MomentaryStatus:
  // 'success' = landed/on, 'ended' = off, no 'failure' case exists yet.
  import type { StatusEffectsDto } from '$lib/tauri/api';
  import { CC_SIZE_CLASSES, type CcSize } from './ccSize';

  let { status, size = 'small' }: { status: StatusEffectsDto | null; size?: CcSize } = $props();

  type CcEffect = { key: string; label: string; active: boolean };

  const effects = $derived(
    (
      [
        ['root', 'Root', status?.root],
        ['stun', 'Stun', status?.stun],
        ['fear', 'Fear', status?.fear],
      ] as const
    ).map(([key, label, m]): CcEffect => ({ key, label, active: m?.outcome === 'success' })),
  );
</script>

<div class="flex {CC_SIZE_CLASSES[size].gap}">
  {#each effects as e (e.key)}
    <span
      class="flex flex-1 items-center justify-center rounded-sm border font-medium uppercase tracking-wide transition-colors {CC_SIZE_CLASSES[
        size
      ].square} {e.active ? 'border-bad bg-bad text-background' : 'border-border text-muted-foreground'}"
      title="{e.label}: {e.active ? 'active' : 'off'}"
    >
      {e.label}
    </span>
  {/each}
</div>
