<script lang="ts">
  // why: timed-effect awareness -- Charm/Invisibility are continuous
  // states (shown as long as active, FADING is the real early warning
  // before it ends), Hide/Sneak are one-shot attempt outcomes (flashed
  // for FLASH_MS then cleared). Same house rules as DpsMeterWidget: flat
  // panel, no continuous CSS animation on a value -- color/text changes
  // are discrete state, not eased. Bare rows, no panel/background of its
  // own; SkillTrackerWidget's outer div owns the one shared panel for
  // every section.
  //
  // Each row is a real member of the same tracked_skills list any other
  // skill/spell lives in -- "Charm"/"Invisible"/"Hide"/"Sneak" start
  // present by default (see preferences.rs's own
  // default_tracked_skills), not hardcoded-forever the way an earlier
  // version had them; `tracked` is just that list, and a row this
  // component would otherwise show still needs its own name in it.
  import type { StatusEffectsDto } from '$lib/tauri/api';

  let { status, tracked }: { status: StatusEffectsDto | null; tracked: string[] } = $props();

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
    if (c.active) return { key: 'Charm', label: `Charm: ACTIVE (${c.who})`, tone: 'good' as const, blink: false };
    return { key: 'Charm', label: `Charm: Broke (${c.who})`, tone: 'bad' as const, blink: nowMs - c.since_ms < CHARM_BLINK_MS };
  });

  const invisRow = $derived.by(() => {
    const s = status?.invis;
    if (!s) return null;
    if (s.active && s.fading) return { key: 'Invisible', label: 'Invisible: FADING', tone: 'warn' as const, blink: false };
    if (s.active) return { key: 'Invisible', label: 'Invisible: ACTIVE', tone: 'good' as const, blink: false };
    if (recent(s.since_ms)) return { key: 'Invisible', label: 'Invisible: ENDED', tone: 'bad' as const, blink: false };
    return null;
  });

  function momentaryRow(key: string, m: { outcome: 'success' | 'failure' | 'ended'; since_ms: number } | null | undefined) {
    if (!m || !recent(m.since_ms)) return null;
    if (m.outcome === 'success') return { key, label: `${key}: SUCCESS`, tone: 'good' as const, blink: false };
    if (m.outcome === 'failure') return { key, label: `${key}: FAILURE`, tone: 'bad' as const, blink: false };
    return { key, label: `${key}: ENDED`, tone: 'dim' as const, blink: false };
  }

  const hideRow = $derived(momentaryRow('Hide', status?.hide));
  const sneakRow = $derived(momentaryRow('Sneak', status?.sneak));

  const rows = $derived(
    [charmRow, invisRow, hideRow, sneakRow].filter((r): r is NonNullable<typeof r> => r !== null && tracked.includes(r.key)),
  );

  const toneClass = { good: 'text-good', bad: 'text-bad', warn: 'text-caution', dim: 'text-white/50' } as const;
</script>

{#if !rows.length}
  <p class="text-white/70">no active effects</p>
{:else}
  {#each rows as r (r.label)}
    <div class="rounded-sm px-1 font-medium {toneClass[r.tone]} {r.blink ? 'charm-broke-blink' : ''}">{r.label}</div>
  {/each}
{/if}

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
