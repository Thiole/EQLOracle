<script lang="ts">
  // why: combined overlay widget, three sections in one window/panel:
  // 1. status effects (Charm/Invis/Hide/Sneak) -- real members of the
  //    same tracked_skills list as any other skill, just on by default
  //    (see preferences.rs's default_tracked_skills), removable the
  //    same way.
  // 2. skill cooldowns (Kick/Bash/...) -- only the ones picked in
  //    Settings, "select skills to track".
  // 3. target effects -- "Target: <name>" header, one icon+timer per
  //    tracked spell effect on it. Real spell-icon art (same assets
  //    SpellbookBuilder renders, see ICON_BASE) when the backend
  //    resolved one; a compact 2-letter badge falls back otherwise.
  //    Player-selected, its own SEPARATE trackedTargetEffectNames list,
  //    not trackedSkillNames -- a spell tracked here never gets its own
  //    cooldown row in section 3, only shows against the current
  //    target. The backend still observes everything (a spell added
  //    mid-fight shows its real history immediately); only what's
  //    rendered here is opt-in.
  //
  // CC status (Root/Stun/Fear) is a SEPARATE overlay widget
  // (CCTrackerWidget.svelte, its own window) -- not a section here. It
  // used to be embedded at this panel's top; moved out to its own
  // widget, same footing as DPS meter/Drop Watch (own on/off, own
  // opacity, own tiny window), since it's glanceable battle-data that
  // doesn't belong sized/positioned together with cooldowns.
  import type { StatusEffectsDto, SkillStatusDto, TargetEffectsDto, SpellCheckDto } from '$lib/tauri/api';
  import { ICON_BASE } from '$lib/character/constants';
  import StatusEffectsWidget from './StatusEffectsWidget.svelte';
  import { logClockNowMs } from './logClock';

  let {
    status,
    skills,
    trackedSkillNames,
    trackedTargetEffectNames,
    targetEffects,
    spellCheck = null,
    opacity,
    overallOpacity,
  }: {
    status: StatusEffectsDto | null;
    skills: SkillStatusDto[];
    trackedSkillNames: string[];
    trackedTargetEffectNames: string[];
    targetEffects: TargetEffectsDto | null;
    spellCheck?: SpellCheckDto | null;
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

  /** why: flash a warning BEFORE a countdown hits zero, not just after.
   * 10s: long enough to react (reapply a slow, back off a DoT target),
   * short enough it only fires once genuinely about to lapse. */
  const EXPIRE_WARN_MS = 10_000;

  /** why: states a target effect can be in -- failed/resisted: flash
   * (hard invert), 0:00. landed and timed out (no wear-off confirmation
   * for most of these -- see targeteffects.rs's doc): flash, 0:00, but
   * keep showing it until the target clears. landed, running, inside
   * EXPIRE_WARN_MS: live countdown plus a milder outline-only warning
   * flash. landed and comfortably running: just a live countdown. */
  function targetEffectState(e: { landed: boolean; ready_at_ms: number | null }) {
    if (!e.landed) return { label: '0:00', flash: true, expiring: false };
    if (e.ready_at_ms !== null && nowMs >= e.ready_at_ms) return { label: '0:00', flash: true, expiring: false };
    if (e.ready_at_ms !== null) {
      const remaining = e.ready_at_ms - nowMs;
      return { label: fmtCountdown(remaining), flash: false, expiring: remaining <= EXPIRE_WARN_MS };
    }
    return { label: '', flash: false, expiring: false };
  }

  const toneClass = { good: 'text-good', warn: 'text-caution' } as const;
</script>

<!-- why: bolder base weight + a dark shadow, inherited by every
     section below (status effects, cooldowns, target effects alike) --
     text stays legible against whatever's behind it once background
     opacity goes to 0, not just this panel's fill. Same treatment as
     DpsMeterWidget's outer div.

     Panel background is the theme's own --background, not a fixed
     value -- see DpsMeterWidget's doc on why color-mix, not a literal
     rgba(), lets a THEME color still take a variable alpha. -->
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

  <!-- why: same SpellPerf hint the DPS meter carries, phrased for this
       widget: "partial resist N%" = how far recent landings sit under
       the invocation-matched baseline. Appears only while struggling. -->
  {#if spellCheck && spellCheck.struggling.length}
    <div class="flex flex-col gap-0.5 border-t border-foreground/10 pt-1.5">
      {#each spellCheck.struggling as sc (sc.name)}
        <div
          class="font-medium text-bad"
          title="recent avg hit {Math.round(sc.recent_avg)} vs {sc.matched
            ? `${spellCheck.invocation ?? 'same-invocation'} baseline`
            : 'session norm'} {Math.round(sc.baseline)}"
        >
          {sc.name} — partial resist {Math.round((1 - sc.ratio) * 100)}%
        </div>
      {/each}
    </div>
  {/if}

  {#if targetEffects?.target && visibleTargetEffects.length}
    {@const anyExpiring = visibleTargetEffects.some((e) => targetEffectState(e).expiring)}
    <div class="flex flex-col gap-1 border-t border-foreground/10 pt-1.5">
      <div class="truncate font-medium {anyExpiring ? 'target-name-expiring text-caution' : 'text-foreground'}">Target: {targetEffects.target}</div>
      <div class="flex flex-wrap gap-2">
        {#each visibleTargetEffects as e (e.spell)}
          {@const st = targetEffectState(e)}
          <div class="flex flex-col items-center gap-0.5" title={e.spell}>
            <div
              class="flex size-7 items-center justify-center overflow-hidden rounded-sm bg-background/60 text-[10px] font-bold tracking-wide {st.flash
                ? 'target-effect-blink'
                : st.expiring
                  ? 'target-effect-expiring text-foreground/90'
                  : 'text-foreground/90'}"
            >
              {#if e.icon}
                <img src={ICON_BASE + encodeURIComponent(e.icon)} alt="" class="size-full object-cover" />
              {:else}
                {abbrev(e.spell)}
              {/if}
            </div>
            <div class="font-mono text-[10px] tabular-nums {st.flash ? 'text-bad' : st.expiring ? 'text-caution' : 'text-muted-foreground'}">{st.label}</div>
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

  /* why: distinct from the hard target-effect-blink above ("already
     over" -- full background invert). Still alive, just running low: a
     pulsing outline ring, not a background invert, so the two states
     never look the same at a glance. */
  .target-effect-expiring {
    animation: target-effect-expiring-flash 0.5s steps(1, end) infinite;
  }
  @keyframes target-effect-expiring-flash {
    50% {
      outline: 2px solid var(--caution);
      outline-offset: 1px;
    }
  }

  /* why: same idea applied to the "Target: <name>" header -- a plain
     color toggle, not an outline (no fixed-size box to ring). */
  .target-name-expiring {
    animation: target-name-expiring-flash 0.5s steps(1, end) infinite;
  }
  @keyframes target-name-expiring-flash {
    50% {
      opacity: 0.55;
    }
  }
</style>
