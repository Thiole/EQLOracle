<script lang="ts">
  // why: combined overlay widget, three sections in one window/panel --
  // Spencer's own spec:
  // 1. status effects (Charm/Invis/Hide/Sneak) -- real members of the
  //    same tracked_skills list as any other skill, just on by default
  //    (see preferences.rs's own default_tracked_skills) instead of
  //    opt-in, and removable the same way.
  // 2. skill cooldowns (Kick/Bash/...) -- only the ones picked in
  //    Settings, "select skills to track".
  // 3. target effects -- "Target: <name>" header, one icon+timer per
  //    tracked spell effect on it. Real spell-icon art (same assets
  //    SpellbookBuilder already renders, see ICON_BASE) when the backend
  //    resolved one; a compact 2-letter badge falls back for anything
  //    unrecognized. Player-selected, but its own SEPARATE
  //    trackedTargetEffectNames list, not trackedSkillNames -- Spencer's
  //    correction, twice: first "should be player selected, not auto
  //    selected", then "dont do spell tracking for 'ready' ... maybe we
  //    need a separate list for 'per target'". A spell tracked here
  //    never gets its own cooldown row in section 2, only ever shows up
  //    against the current target. The backend still observes
  //    everything (so a spell added mid-fight shows its real history
  //    immediately, not just future casts); only what's rendered here
  //    is opt-in.
  import type { StatusEffectsDto, SkillStatusDto, TargetEffectsDto } from '$lib/tauri/api';
  import { ICON_BASE } from '$lib/character/constants';
  import StatusEffectsWidget from './StatusEffectsWidget.svelte';
  import { logClockNowMs } from './logClock';

  let {
    status,
    skills,
    trackedSkillNames,
    trackedTargetEffectNames,
    targetEffects,
    opacity,
    overallOpacity,
  }: {
    status: StatusEffectsDto | null;
    skills: SkillStatusDto[];
    trackedSkillNames: string[];
    trackedTargetEffectNames: string[];
    targetEffects: TargetEffectsDto | null;
    opacity: number;
    // why: the SEPARATE "everything" fade -- see DpsMeterWidget.svelte's
    // own doc, same idea, applied to this widget's own outer element
    // (covers all three sections -- status effects, cooldowns, target
    // effects -- since none of them own a panel of their own)
    overallOpacity: number;
  } = $props();

  // why: NOT Date.now() -- see logClock.ts's own doc. Backend since_ms/
  // ready_at_ms are the log's own "naive local time" clock, not a real
  // UTC epoch; comparing against a real one would skew every countdown
  // here by the machine's own UTC offset.
  let nowMs = $state(logClockNowMs());
  $effect(() => {
    const id = setInterval(() => (nowMs = logClockNowMs()), 250);
    return () => clearInterval(id);
  });

  const visibleSkills = $derived(skills.filter((s) => trackedSkillNames.includes(s.skill)));

  // why: a target-effect's own `spell` can carry a live per-character
  // rank suffix the tracked list never does (added via Spellbook's
  // search against the catalog's own base name) -- see
  // targeteffects.rs's own doc on the two attribution paths disagreeing
  // on this. Checked both ways rather than needing the server's own
  // PROTECTED_SPELL_NAMES list client-side: a name that's genuinely
  // just roman numerals (no real un-suffixed catalog entry) would only
  // ever appear in trackedTargetEffectNames under its own full name
  // anyway, so the stripped fallback never false-matches it.
  function stripRank(name: string): string {
    const parts = name.split(' ');
    const tail = parts[parts.length - 1];
    if (parts.length > 1 && /^[IVXLCDM]+$/.test(tail)) {
      return parts.slice(0, -1).join(' ');
    }
    return name;
  }
  const visibleTargetEffects = $derived(
    (targetEffects?.effects ?? []).filter(
      (e) => trackedTargetEffectNames.includes(e.spell) || trackedTargetEffectNames.includes(stripRank(e.spell)),
    ),
  );

  function fmtCountdown(ms: number): string {
    const secs = Math.max(0, Math.ceil(ms / 1000));
    const m = Math.floor(secs / 60);
    const s = secs % 60;
    return m > 0 ? `${m}:${String(s).padStart(2, '0')}` : `${s}s`;
  }

  function skillRow(s: SkillStatusDto) {
    // why: ready_at_ms is already a real absolute deadline (resolved
    // server-side as max(reuse, recovery), see skilltracker.rs's own
    // doc), so a smooth local countdown against nowMs is precise -- same
    // as targetEffectState's own ready_at_ms countdown
    const missed = s.last_outcome === 'avoided' ? ' (missed)' : '';
    if (s.ready_at_ms === null) {
      return { text: `${s.skill}: READY${missed}`, tone: 'good' as const };
    }
    const ready = nowMs >= s.ready_at_ms;
    const label = ready ? 'READY' : fmtCountdown(s.ready_at_ms - nowMs);
    return { text: `${s.skill}: ${label}${missed}`, tone: ready ? ('good' as const) : ('warn' as const) };
  }

  function abbrev(name: string): string {
    const words = name.split(/\s+/).filter((w) => /[A-Za-z]/.test(w[0] ?? ''));
    if (words.length >= 2) return (words[0][0] + words[1][0]).toUpperCase();
    return name.slice(0, 2).toUpperCase();
  }

  /** why: Spencer's own spec for the three real states a target effect can be in --
   * failed/resisted: flash, 0:00. landed and timed out (no wear-off confirmation exists
   * for most of these -- see targeteffects.rs's own doc): flash, 0:00, but keep showing it,
   * not drop it, until the target itself clears. landed and still running: a live countdown. */
  function targetEffectState(e: { landed: boolean; ready_at_ms: number | null }) {
    if (!e.landed) return { label: '0:00', flash: true };
    if (e.ready_at_ms !== null && nowMs >= e.ready_at_ms) return { label: '0:00', flash: true };
    if (e.ready_at_ms !== null) return { label: fmtCountdown(e.ready_at_ms - nowMs), flash: false };
    return { label: '', flash: false };
  }

  const toneClass = { good: 'text-good', warn: 'text-caution' } as const;
</script>

<!-- why: Spencer's own ask -- "make text a bit darker/bolder by
     default, so if only background is removed, its more readable".
     Bolder base weight + a dark shadow, inherited by every section
     below (status effects, cooldowns, target effects alike, none of
     them own a panel of their own) -- text stays legible against
     whatever's actually behind it once background opacity goes to 0,
     not just this panel's own dark fill. Same treatment as
     DpsMeterWidget's own outer div.

     Panel background is the theme's own --background now (Spencer's
     own ask: "ui overlay theme should match") -- see DpsMeterWidget's
     own doc on why color-mix, not a literal rgba(), is what lets a
     THEME color still take a variable alpha. -->
<div
  class="flex flex-col gap-1.5 rounded-md p-2 text-[12px] font-semibold"
  style:background-color="color-mix(in srgb, var(--background) {opacity * 100}%, transparent)"
  style:opacity={overallOpacity}
  style:text-shadow="0 1px 2px rgba(0, 0, 0, 0.9), 0 0px 4px rgba(0, 0, 0, 0.6)"
>
  <StatusEffectsWidget {status} tracked={trackedSkillNames} />

  {#if visibleSkills.length}
    <div class="flex flex-col gap-0.5 border-t border-foreground/10 pt-1.5">
      {#each visibleSkills as s (s.skill)}
        {@const row = skillRow(s)}
        <div class="font-medium {toneClass[row.tone]}">{row.text}</div>
      {/each}
    </div>
  {/if}

  {#if targetEffects?.target && visibleTargetEffects.length}
    <div class="flex flex-col gap-1 border-t border-foreground/10 pt-1.5">
      <div class="truncate font-medium text-foreground">Target: {targetEffects.target}</div>
      <div class="flex flex-wrap gap-2">
        {#each visibleTargetEffects as e (e.spell)}
          {@const st = targetEffectState(e)}
          <div class="flex flex-col items-center gap-0.5" title={e.spell}>
            <div class="flex size-7 items-center justify-center overflow-hidden rounded-sm bg-background/60 text-[10px] font-bold tracking-wide {st.flash ? 'target-effect-blink' : 'text-foreground/90'}">
              {#if e.icon}
                <img src={ICON_BASE + encodeURIComponent(e.icon)} alt="" class="size-full object-cover" />
              {:else}
                {abbrev(e.spell)}
              {/if}
            </div>
            <div class="font-mono text-[10px] tabular-nums {st.flash ? 'text-bad' : 'text-muted-foreground'}">{st.label}</div>
          </div>
        {/each}
      </div>
    </div>
  {/if}
</div>

<style>
  /* why: same hard on/off blink as StatusEffectsWidget's own charm-broke
     alert -- a failed/resisted cast or an uncertain-expired timer both
     get the same real attention-grabbing treatment, not a color alone.
     "off" state is the theme's own --background at a fixed alpha (was a
     literal rgba(0,0,0,0.4)); "on" state's text is the theme's own
     --background too, not a hardcoded near-black -- --bad is a bright
     semantic color across every theme here so a theme background
     reliably reads dark enough against it either way, and this way
     nothing in the flash is a raw color the theme doesn't own. */
  .target-effect-blink {
    animation: target-effect-flash 0.4s steps(1, end) infinite;
  }
  @keyframes target-effect-flash {
    0%,
    100% {
      background-color: color-mix(in srgb, var(--background) 40%, transparent);
      color: var(--bad);
    }
    50% {
      background-color: var(--bad);
      color: var(--background);
    }
  }
</style>
