<script lang="ts">
  // why: timed-effect awareness -- Charm/Invisibility are continuous
  // states (shown as long as active, FADING is the real early warning
  // before it ends), Hide/Sneak are one-shot attempt outcomes (flashed
  // for FLASH_MS then cleared). Same house rules as DpsMeterWidget: flat
  // panel, no continuous CSS animation on a value -- color/text changes
  // are discrete state, not eased.
  import type { StatusEffectsDto } from '$lib/tauri/api';

  let { status, opacity }: { status: StatusEffectsDto | null; opacity: number } = $props();

  /** why: how long a one-shot outcome (hide/sneak/invis-ended) stays
   * visible before it's just stale news, not a real overlay-worthy fact */
  const FLASH_MS = 8000;
  /** why: charm breaking is the one state worth a real blink, not just a
   * color -- a charmed mob turning hostile mid-fight is genuinely
   * dangerous. Blinks for this long, then settles into a plain, still
   * red, still-shown "Broke" -- see charmRow's own doc for why it never
   * just disappears like the others. */
  const CHARM_BLINK_MS = 2000;

  let nowMs = $state(Date.now());
  $effect(() => {
    const id = setInterval(() => (nowMs = Date.now()), 200);
    return () => clearInterval(id);
  });

  const recent = (sinceMs: number) => nowMs - sinceMs < FLASH_MS;

  // why: unlike every other row here, this one is deliberately NOT
  // time-gated to disappear -- Spencer's own ask: "maintain the line"
  // once it breaks, don't let it quietly vanish after FLASH_MS the way
  // hide/sneak do. It stays Broke until a new charm actually lands.
  const charmRow = $derived.by(() => {
    const c = status?.charm;
    if (!c) return null;
    if (c.active) return { label: `Charm: ACTIVE (${c.who})`, tone: 'good' as const, blink: false };
    return { label: `Charm: Broke (${c.who})`, tone: 'bad' as const, blink: nowMs - c.since_ms < CHARM_BLINK_MS };
  });

  const invisRow = $derived.by(() => {
    const s = status?.invis;
    if (!s) return null;
    if (s.active && s.fading) return { label: 'Invisible: FADING', tone: 'warn' as const, blink: false };
    if (s.active) return { label: 'Invisible: ACTIVE', tone: 'good' as const, blink: false };
    if (recent(s.since_ms)) return { label: 'Invisible: ENDED', tone: 'bad' as const, blink: false };
    return null;
  });

  function momentaryRow(label: string, m: { outcome: 'success' | 'failure' | 'ended'; since_ms: number } | null | undefined) {
    if (!m || !recent(m.since_ms)) return null;
    if (m.outcome === 'success') return { label: `${label}: SUCCESS`, tone: 'good' as const, blink: false };
    if (m.outcome === 'failure') return { label: `${label}: FAILURE`, tone: 'bad' as const, blink: false };
    return { label: `${label}: ENDED`, tone: 'dim' as const, blink: false };
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
      <div class="rounded-sm px-1 font-medium {toneClass[r.tone]} {r.blink ? 'charm-broke-blink' : ''}">{r.label}</div>
    {/each}
  {/if}
</div>

<style>
  /* why: the one row worth a real blink, not just a color -- Spencer's
     own ask, charm breaking mid-fight is genuinely dangerous. Hard
     on/off steps, not an eased pulse -- an alert, not a decoration.
     Inverts (solid red panel, light text) on the "on" beat, plain red
     text on the "off" beat -- settles into the "off" look once the
     blink window ends (see charmRow's own doc), never mid-invert. */
  .charm-broke-blink {
    animation: charm-broke-flash 0.4s steps(1, end) 5;
  }
  @keyframes charm-broke-flash {
    0%,
    100% {
      background-color: transparent;
      color: var(--bad);
    }
    50% {
      background-color: var(--bad);
      color: #0a0b0d;
    }
  }
</style>
