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
  //    unrecognized. Player-selected, same trackedSkillNames list as
  //    cooldowns above -- Spencer's own correction: this used to show
  //    every DoT/debuff landed on the target unconditionally, "should
  //    be player selected, not auto selected". The backend still
  //    observes everything (so a spell added mid-fight shows its real
  //    history immediately, not just future casts); only what's
  //    rendered here is opt-in now.
  import type { StatusEffectsDto, SkillStatusDto, TargetEffectsDto } from '$lib/tauri/api';
  import { ICON_BASE } from '$lib/character/constants';
  import StatusEffectsWidget from './StatusEffectsWidget.svelte';

  let {
    status,
    skills,
    trackedSkillNames,
    targetEffects,
    opacity,
  }: {
    status: StatusEffectsDto | null;
    skills: SkillStatusDto[];
    trackedSkillNames: string[];
    targetEffects: TargetEffectsDto | null;
    opacity: number;
  } = $props();

  let nowMs = $state(Date.now());
  $effect(() => {
    const id = setInterval(() => (nowMs = Date.now()), 250);
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
  // ever appear in trackedSkillNames under its own full name anyway, so
  // the stripped fallback never false-matches it.
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
      (e) => trackedSkillNames.includes(e.spell) || trackedSkillNames.includes(stripRank(e.spell)),
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

<div class="flex flex-col gap-1.5 rounded-md p-2 text-[12px]" style:background-color="rgba(10, 11, 13, {opacity})">
  <StatusEffectsWidget {status} tracked={trackedSkillNames} />

  {#if visibleSkills.length}
    <div class="flex flex-col gap-0.5 border-t border-white/10 pt-1.5">
      {#each visibleSkills as s (s.skill)}
        {@const row = skillRow(s)}
        <div class="font-medium {toneClass[row.tone]}">{row.text}</div>
      {/each}
    </div>
  {/if}

  {#if targetEffects?.target && visibleTargetEffects.length}
    <div class="flex flex-col gap-1 border-t border-white/10 pt-1.5">
      <div class="truncate font-medium text-white">Target: {targetEffects.target}</div>
      <div class="flex flex-wrap gap-2">
        {#each visibleTargetEffects as e (e.spell)}
          {@const st = targetEffectState(e)}
          <div class="flex flex-col items-center gap-0.5" title={e.spell}>
            <div class="flex size-7 items-center justify-center overflow-hidden rounded-sm bg-black/40 text-[10px] font-bold tracking-wide {st.flash ? 'target-effect-blink' : 'text-white/90'}">
              {#if e.icon}
                <img src={ICON_BASE + encodeURIComponent(e.icon)} alt="" class="size-full object-cover" />
              {:else}
                {abbrev(e.spell)}
              {/if}
            </div>
            <div class="font-mono text-[10px] tabular-nums {st.flash ? 'text-bad' : 'text-white/70'}">{st.label}</div>
          </div>
        {/each}
      </div>
    </div>
  {/if}
</div>

<style>
  /* why: same hard on/off blink as StatusEffectsWidget's own charm-broke
     alert -- a failed/resisted cast or an uncertain-expired timer both
     get the same real attention-grabbing treatment, not a color alone */
  .target-effect-blink {
    animation: target-effect-flash 0.4s steps(1, end) infinite;
  }
  @keyframes target-effect-flash {
    0%,
    100% {
      background-color: rgba(0, 0, 0, 0.4);
      color: var(--bad);
    }
    50% {
      background-color: var(--bad);
      color: #0a0b0d;
    }
  }
</style>
