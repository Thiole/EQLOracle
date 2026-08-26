<script lang="ts">
  // why: timed-effect awareness -- Charm/Invisibility are continuous
  // states (shown as long as active, FADING is the real early warning
  // before it ends), Hide/Sneak are one-shot attempt outcomes (flashed
  // for FLASH_MS then cleared). Same house rules as DpsMeterWidget: flat
  // panel, no continuous CSS animation on a value -- color/text changes
  // are discrete state, not eased.
  import type { StatusEffectsDto } from '$lib/tauri/api';

  let { status, opacity }: { status: StatusEffectsDto | null; opacity: number } = $props();

  /** why: how long a one-shot outcome (hide/sneak/charm-broke/invis-ended)
   * stays visible before it's just stale news, not a real overlay-worthy fact */
  const FLASH_MS = 8000;

  let nowMs = $state(Date.now());
  $effect(() => {
    const id = setInterval(() => (nowMs = Date.now()), 500);
    return () => clearInterval(id);
  });

  const recent = (sinceMs: number) => nowMs - sinceMs < FLASH_MS;

  const charmRow = $derived.by(() => {
    const c = status?.charm;
    if (!c) return null;
    if (c.active) return { label: `Charm: ACTIVE (${c.who})`, tone: 'good' as const };
    if (recent(c.since_ms)) return { label: `Charm: BROKE (${c.who})`, tone: 'bad' as const };
    return null;
  });

  const invisRow = $derived.by(() => {
    const s = status?.invis;
    if (!s) return null;
    if (s.active && s.fading) return { label: 'Invisible: FADING', tone: 'warn' as const };
    if (s.active) return { label: 'Invisible: ACTIVE', tone: 'good' as const };
    if (recent(s.since_ms)) return { label: 'Invisible: ENDED', tone: 'bad' as const };
    return null;
  });

  function momentaryRow(label: string, m: { outcome: 'success' | 'failure' | 'ended'; since_ms: number } | null | undefined) {
    if (!m || !recent(m.since_ms)) return null;
    if (m.outcome === 'success') return { label: `${label}: SUCCESS`, tone: 'good' as const };
    if (m.outcome === 'failure') return { label: `${label}: FAILURE`, tone: 'bad' as const };
    return { label: `${label}: ENDED`, tone: 'dim' as const };
  }

  const hideRow = $derived(momentaryRow('Hide', status?.hide));
  const sneakRow = $derived(momentaryRow('Sneak', status?.sneak));

  const rows = $derived([charmRow, invisRow, hideRow, sneakRow].filter((r) => r !== null));

  const toneClass = { good: 'text-good', bad: 'text-bad', warn: 'text-caution', dim: 'text-white/50' } as const;
</script>

<div class="flex flex-col gap-1 rounded-md p-2 text-[12px]" style:background-color="rgba(10, 11, 13, {opacity})">
  {#if !rows.length}
    <p class="text-white/70">no active effects</p>
  {:else}
    {#each rows as r (r.label)}
      <div class="font-medium {toneClass[r.tone]}">{r.label}</div>
    {/each}
  {/if}
</div>
