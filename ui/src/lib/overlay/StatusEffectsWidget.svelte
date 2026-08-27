<script lang="ts">
  // why: timed-effect awareness -- Charm/Invisibility are continuous
  // states (shown as long as active, FADING is the real early warning
  // before it ends). Hide/Sneak are one-shot attempt outcomes, but
  // persistent like Charm now, not auto-hidden -- real bug, caught
  // live: an 8s flash window is gone by the time anyone's actually
  // looking at the overlay ("i just hid and sneaked and its not
  // showing up" -- the data was real, the window was just too short to
  // ever see). Blinks briefly on the moment itself, then settles into a
  // plain, still-shown result until a newer attempt or a real "no
  // longer hidden" line replaces it. Same house rules as DpsMeterWidget: flat
  // panel, no continuous CSS animation on a value -- color/text changes
  // are discrete state, not eased. Bare rows, no panel/background of its
  // own; SkillTrackerWidget's outer div owns the one shared panel for
  // every section.
  //
  // Each row is a real member of the same tracked_skills list any other
  // skill/spell lives in -- "Charmed"/"Invisible"/"Hide"/"Sneak" start
  // present by default (see preferences.rs's own
  // default_tracked_skills), not hardcoded-forever the way an earlier
  // version had them; `tracked` is just that list, and a row this
  // component would otherwise show still needs its own name in it.
  // "Charmed" not "Charm" -- real bug, caught live: a real Enchanter
  // spell is named exactly "Charm", so a cooldown entry for that spell
  // collided with this row's own key and showed a nonsense "Charm:
  // READY" row alongside the real status row.
  import type { StatusEffectsDto } from '$lib/tauri/api';
  import { logClockNowMs } from './logClock';

  let { status, tracked }: { status: StatusEffectsDto | null; tracked: string[] } = $props();

  /** why: how long invis-ended stays visible before it's just stale news
   * -- the one row here that's still genuinely a fading flash, not a
   * persistent result (unlike Charm/Hide/Sneak, see their own docs) */
  const FLASH_MS = 8000;
  /** why: the moment itself is worth a real blink, not just a color --
   * charm breaking mid-fight is dangerous, and hide/sneak succeeding or
   * failing is the one piece of information that action even happened.
   * Blinks for this long, then settles into a plain, still-shown result. */
  const BLINK_MS = 2000;

  // why: NOT Date.now() -- see logClock.ts's own doc. since_ms here is
  // the log's own "naive local time" clock, not a real UTC epoch;
  // comparing against a real one would skew recent()/the blink windows
  // by the machine's own UTC offset (real bug, same shape SkillTracker
  // Widget's own cooldowns/target-effects countdowns had).
  let nowMs = $state(logClockNowMs());
  $effect(() => {
    const id = setInterval(() => (nowMs = logClockNowMs()), 200);
    return () => clearInterval(id);
  });

  const recent = (sinceMs: number) => nowMs - sinceMs < FLASH_MS;

  // why: unlike every other row here, this one is deliberately NOT
  // time-gated to disappear once it breaks, unlike hide/sneak. It
  // stays Broke until a new charm actually lands.
  const charmRow = $derived.by(() => {
    const c = status?.charm;
    if (!c) return null;
    // why: key is "Charmed" not "Charm" -- see this file's own doc for the
    // real spell-name collision this avoids; the displayed label still
    // reads "Charm", only the tracked-list identity changed
    if (c.active) return { key: 'Charmed', label: `Charm: ACTIVE (${c.who})`, tone: 'good' as const, blink: false };
    return { key: 'Charmed', label: `Charm: Broke (${c.who})`, tone: 'bad' as const, blink: nowMs - c.since_ms < BLINK_MS };
  });

  const invisRow = $derived.by(() => {
    const s = status?.invis;
    if (!s) return null;
    if (s.active && s.fading) return { key: 'Invisible', label: 'Invisible: FADING', tone: 'warn' as const, blink: false };
    if (s.active) return { key: 'Invisible', label: 'Invisible: ACTIVE', tone: 'good' as const, blink: false };
    if (recent(s.since_ms)) return { key: 'Invisible', label: 'Invisible: ENDED', tone: 'bad' as const, blink: false };
    return null;
  });

  // why: real bug, caught live -- this used to auto-hide after FLASH_MS
  // (8s), long gone by the time anyone actually checks the overlay
  // ("i just hid and sneaked and its not showing up"). Now persistent
  // like charmRow above: no recent() gate, just a blink on the moment.
  function momentaryRow(key: string, m: { outcome: 'success' | 'failure' | 'ended'; since_ms: number } | null | undefined) {
    if (!m) return null;
    const blink = nowMs - m.since_ms < BLINK_MS;
    if (m.outcome === 'success') return { key, label: `${key}: SUCCESS`, tone: 'good' as const, blink };
    if (m.outcome === 'failure') return { key, label: `${key}: FAILURE`, tone: 'bad' as const, blink };
    return { key, label: `${key}: ENDED`, tone: 'dim' as const, blink };
  }

  const hideRow = $derived(momentaryRow('Hide', status?.hide));
  const sneakRow = $derived(momentaryRow('Sneak', status?.sneak));

  const rows = $derived(
    [charmRow, invisRow, hideRow, sneakRow].filter((r): r is NonNullable<typeof r> => r !== null && tracked.includes(r.key)),
  );

  const toneClass = { good: 'text-good', bad: 'text-bad', warn: 'text-caution', dim: 'text-muted-foreground' } as const;
</script>

{#if !rows.length}
  <p class="text-muted-foreground">no active effects</p>
{:else}
  {#each rows as r (r.label)}
    <div class="rounded-sm px-1 font-medium {toneClass[r.tone]} {r.blink ? `status-blink status-blink-${r.tone}` : ''}">{r.label}</div>
  {/each}
{/if}

<style>
  /* why: the moment itself is worth a real blink, not just a color --
     charm breaking mid-fight, or hide/sneak's own success/failure/ended,
     shared by every row in this file (see BLINK_MS's own doc). Hard
     on/off steps, not an eased pulse -- an alert, not a decoration.
     Inverts (solid panel, dark text) on the "on" beat; leaves color
     alone on the "off" beat so it settles right back into the row's own
     toneClass color once the blink window ends, never mid-invert.
     One class per tone -- a SUCCESS row blinks its own good color, not
     always red the way a charm-only version could get away with.
     --blink-color and the inverted text are theme tokens, not raw
     rgba()/hex values, so overlay theme matches the main window. */
  .status-blink {
    animation: status-flash 0.4s steps(1, end) 5;
  }
  .status-blink-good {
    --blink-color: var(--good);
  }
  .status-blink-bad {
    --blink-color: var(--bad);
  }
  .status-blink-dim {
    --blink-color: color-mix(in srgb, var(--foreground) 85%, transparent);
  }
  @keyframes status-flash {
    50% {
      background-color: var(--blink-color);
      color: var(--background);
    }
  }
</style>
