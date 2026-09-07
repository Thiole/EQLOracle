<script lang="ts">
  // why: composes data other modules already fetch (character, zone, mob
  // history) plus overview.rs's own dedicated session-rate stats -- a
  // real backend module (get_session) that already existed, wired to no
  // frontend until now. Landing page: "what's going on right now", link
  // out to the module that actually owns each answer.
  import { Card, CardContent } from '$lib/components/ui/card';
  import { Badge } from '$lib/components/ui/badge';
  import { Button } from '$lib/components/ui/button';
  import { api, type ZoneContextDto, type MobDto, type GroupBuffsDto } from '$lib/tauri/api';
  import { listen } from '$lib/tauri/invoke';
  import BellOffIcon from '@lucide/svelte/icons/bell-off';
  import { toggleMutedBuffLine } from '$lib/stores/settings';
  import { activeModule } from '$lib/stores/shell';
  import { race, activeClasses, defaultClasses, classConfigurations, loadCharacterModule } from '$lib/stores/character';
  import { session, refreshSession, resetSession, setSessionWindow } from '$lib/stores/session';
  import { displayZoneName, logMsToLocalInput, localInputToLogMs } from '$lib/utils';
  import LootHistory from '$lib/monsters/LootHistory.svelte';

  $effect(() => {
    void loadCharacterModule();
    void refreshSession();
  });

  // why: reported -- "if i buff it doesn't load until i reload the tab".
  // Overview only ever loaded on mount and on the one-shot parse-settled,
  // so a landing page about "what's going on right now" sat on whatever
  // moment it opened at. The catalog (listMobs) stays off this: it is a
  // catalog, not live state.
  $effect(() => {
    const un = listen('parse-tick', () => {
      loadGroupBuffs();
      void refreshSession();
      api.getZoneContext().then((z) => (zoneCtx = z));
    });
    return () => void un.then((f) => f());
  });

  let zoneCtx = $state<ZoneContextDto | null>(null);
  let mobs = $state<MobDto[] | null>(null);
  let resetting = $state(false);
  let lootExpanded = $state(false);

  // why: these load once -- a backfill still running (or a frozen replay)
  // would otherwise leave the card on whatever moment it mounted at, so
  // the same load runs again when the parse settles (see events.ts)
  function loadOverviewData() {
    api.getZoneContext().then((z) => (zoneCtx = z));
    api.listMobs().then((list) => (mobs = list ?? []));
    loadGroupBuffs();
  }
  $effect(() => {
    loadOverviewData();
    const onSettled = () => loadOverviewData();
    window.addEventListener('eqlp:parse-settled', onSettled);
    return () => window.removeEventListener('eqlp:parse-settled', onSettled);
  });

  // why: the same tracker the overlay shows, on the landing page -- what
  // is MISSING only (a covered buff needs no row), each one carrying its
  // own mute. Party rows and your own innates read the same here: a line
  // you could have on and do not.
  let buffs = $state<GroupBuffsDto | null>(null);
  function loadGroupBuffs() {
    api.getGroupBuffs().then((d) => (buffs = d)).catch(() => (buffs = null));
  }
  // why: the LINE is what a mute switches off, the label is the stat it
  // fills -- "not the spell but the slot it fills". A row that is up but
  // upgradeable still needs saying; it names the better line.
  //
  // Split the way the minimal overlay layout reads its verdict, so the
  // card answers what that one word is about: your own self-buffs are
  // "Warning", a groupmate's buffs on you are "Others missing". Party
  // rows name the player expected to cast the line -- "the expected
  // spell line of the players".
  const missingRows = $derived(buffs ? buffs.rows.filter((r) => !r.active || r.upgrade) : []);
  const missingParty = $derived(
    buffs
      ? missingRows
          .filter((r) => r.others)
          .map((r) => ({
            line: r.lines[0]?.line ?? r.label,
            label: r.label,
            who: (r.lines[0]?.casters ?? []).filter((c) => c !== 'You').join(', '),
            was: r.upgrade ? r.active : null,
          }))
      : [],
  );
  // why: a row only YOU can cast belongs with your own self-buffs, not
  // under "expected from your party" -- you are a source like any
  // groupmate, so a party row can name nobody but you
  const missingOwn = $derived(
    buffs
      ? [
          ...buffs.innates
            .filter((i) => !i.active)
            .map((i) => ({ line: i.line, label: i.label, who: '', was: null as string | null })),
          ...missingRows
            .filter((r) => !r.others)
            .map((r) => ({
              line: r.lines[0]?.line ?? r.label,
              label: r.label,
              who: '',
              was: r.upgrade ? r.active : null,
            })),
        ]
      : [],
  );
  const neededBuffs = $derived([...missingOwn, ...missingParty]);
  async function muteLine(line: string) {
    await toggleMutedBuffLine(line);
    loadGroupBuffs();
  }

  // why: "manual override button to set timeframe" -- a start and an
  // optional end as local datetimes; empty end means "now"
  let framing = $state(false);
  let frameStart = $state('');
  let frameEnd = $state('');
  function openFrame() {
    frameStart = $session?.session_start_ms != null ? logMsToLocalInput($session.session_start_ms) : '';
    frameEnd = $session?.session_end_ms != null ? logMsToLocalInput($session.session_end_ms) : '';
    framing = true;
  }
  async function applyFrame() {
    // why: log time, not browser-local epoch -- see utils' fmtLogTime doc;
    // the first cut sent local epoch and the window landed 4 hours off,
    // "setting the timeframe doesn't retroactively build the session"
    const start = frameStart ? localInputToLogMs(frameStart) : null;
    const end = frameEnd ? localInputToLogMs(frameEnd) : null;
    if (start == null) return;
    await setSessionWindow(start, end);
    framing = false;
  }
  async function autoFrame() {
    await setSessionWindow(null, null);
    framing = false;
  }

  async function restart() {
    resetting = true;
    try {
      await resetSession();
    } finally {
      resetting = false;
    }
  }

  function sameClassSet(a: string[], b: string[]): boolean {
    if (a.length !== b.length) return false;
    const sa = [...a].sort();
    const sb = [...b].sort();
    return sa.every((c, i) => c === sb[i]);
  }

  // why: NOT the Character Planner's `levels` store -- that's a manual,
  // edit-by-hand gear-planning value, seeded once and never re-synced to
  // a real "Welcome to level N!" line. This instead reads
  // classConfigurations' own level_range, which class_configurations.rs
  // computes fresh from real level.up lines strictly inside the
  // confirmed configuration's own active zone-visit windows -- so it
  // reflects a ding attributed to the class you were actually on when it
  // happened, not just the file-wide latest level. Several "sessions" of
  // the same trio can exist (a revisit after a long gap splits into its
  // own entry -- see class_configurations' own SESSION_GAP_MS doc); the
  // highest upper bound across all of them is the real, confirmed peak.
  const dominantLevel = $derived(
    ($classConfigurations?.configurations ?? [])
      .filter((c) => sameClassSet(c.classes, $defaultClasses))
      .reduce<number | null>((best, c) => {
        const hi = c.level_range?.[1] ?? null;
        return hi !== null && (best === null || hi > best) ? hi : best;
      }, null),
  );
  const shownLevel = $derived(dominantLevel ?? $session?.current_level ?? null);
  const shownClasses = $derived($defaultClasses.length ? $defaultClasses : $activeClasses);

  const totalKills = $derived((mobs ?? []).reduce((n, m) => n + m.kills, 0));
  const totalPulls = $derived((mobs ?? []).reduce((n, m) => n + m.pulls, 0));
  const topMobs = $derived((mobs ?? []).slice(0, 5));

  function fmtHours(h: number): string {
    return h < 1 ? `${Math.round(h * 60)}m` : `${h.toFixed(1)}h`;
  }

  // why: live "Current session: h:mm:ss" clock at the top of the Session
  // card -- session_duration_ms only refreshes on parse-tick (every few
  // seconds), which would visibly stutter as a clock. Resynced to the
  // real backend value on every refresh (see the $effect below), ticked
  // locally in between so it advances every second regardless.
  let liveDurationMs = $state(0);
  $effect(() => {
    if ($session) liveDurationMs = $session.session_duration_ms;
  });
  $effect(() => {
    const id = setInterval(() => (liveDurationMs += 1000), 1000);
    return () => clearInterval(id);
  });
  function fmtDuration(ms: number): string {
    const totalSec = Math.max(0, Math.floor(ms / 1000));
    const h = Math.floor(totalSec / 3600);
    const m = Math.floor((totalSec % 3600) / 60);
    const s = totalSec % 60;
    const pad = (n: number) => n.toString().padStart(2, '0');
    return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${pad(m)}:${pad(s)}`;
  }

  // why: 9 real tiers, ascending -- see overview.rs's MOTE_TIER_ORDER for
  // why these are colors, not scraped icons (Motes have no icon/stats in
  // the wiki scrape at all, just a bare crafting-component stub)
  const TIER_COLORS: Record<string, string> = {
    Infinitesimal: '#8b93a1',
    Minor: '#7ea6c9',
    Lesser: '#6ec9a0',
    Greater: '#9ed15c',
    Major: '#e0c93c',
    Superior: '#e0973c',
    Grand: '#e0603c',
    Ascendant: '#c964e0',
    Infinite: '#f2d675',
  };

  function goto(module: string) {
    activeModule.set(module);
  }
</script>

<div class="flex flex-col gap-3 p-3">
  {#if lootExpanded}
    <button
      type="button"
      class="self-start text-[11px] text-brand-soft hover:text-primary hover:underline"
      onclick={() => (lootExpanded = false)}
    >
      ← back to Overview
    </button>
    <LootHistory />
  {:else}
    <div class="grid grid-cols-2 items-start gap-3">
      <div class="flex flex-col gap-3">
        <Card class="rounded-sm">
          <CardContent class="px-3 py-2.5">
            <h2 class="stat-figure mb-1.5 text-[18px]">Character</h2>
            {#if shownLevel === null && !shownClasses.length}
              <p class="text-[11px] text-muted-foreground">No class evidence parsed yet this session.</p>
            {:else}
              <p class="text-[12px]">
                {#if $race}{$race} · {/if}level {shownLevel ?? '?'}
                {#if $session?.progress_pct != null}
                  <span class="text-muted-foreground">({$session.progress_pct.toFixed(1)}% to next)</span>
                {/if}
              </p>
              {#if shownClasses.length}
                <p class="mt-0.5 text-[11px] text-muted-foreground">
                  {shownClasses.join(' / ')}{#if !$defaultClasses.length} (not yet confirmed){/if}
                </p>
              {/if}
            {/if}
            <button type="button" class="mt-2 text-[11px] text-brand-soft hover:text-primary hover:underline" onclick={() => goto('character')}>
              open Character →
            </button>
          </CardContent>
        </Card>

        <Card class="rounded-sm">
          <CardContent class="px-3 py-2.5">
            <div class="mb-1.5 flex items-center justify-between">
              <div class="flex items-baseline gap-2">
                <h2 class="stat-figure text-[18px]">Session</h2>
                <span class="font-mono text-[12px] text-muted-foreground">Current session: {fmtDuration(liveDurationMs)}</span>
                {#if $session?.afk}
                  <Badge variant="outline" class="h-5 text-[10px]">afk</Badge>
                {/if}
              </div>
              <div class="flex items-center gap-1">
                <span class="font-mono text-[10px] text-muted-foreground" title="auto: starts after the last 30 minutes with no action by you or your party">
                  {$session?.mode === 'manual' ? 'manual timeframe' : $session?.mode === 'restart' ? 'since restart' : 'auto · 30-min gap'}
                </span>
                <Button size="sm" variant="ghost" class="h-6 text-[11px]" onclick={openFrame} title="Set the session's start and end yourself">
                  set timeframe
                </Button>
                <Button size="sm" variant="ghost" class="h-6 text-[11px]" onclick={restart} disabled={resetting} title="Zero out plat/motes/levels/AA and start counting from right now">
                  {resetting ? 'restarting…' : 'restart'}
                </Button>
              </div>
            </div>
            {#if framing}
              <div class="mb-2 flex flex-wrap items-end gap-2 rounded-sm border border-border bg-muted/30 px-2 py-1.5 text-[11px]">
                <label class="flex flex-col gap-0.5">
                  <span class="text-[10px] uppercase tracking-wide text-muted-foreground">start</span>
                  <input type="text" bind:value={frameStart} placeholder="2026-09-02 15:30" spellcheck="false" class="h-6 w-36 rounded-sm border border-border bg-background px-1 font-mono text-[11px]" />
                </label>
                <label class="flex flex-col gap-0.5">
                  <span class="text-[10px] uppercase tracking-wide text-muted-foreground">end (blank = now)</span>
                  <input type="text" bind:value={frameEnd} placeholder="blank = now" spellcheck="false" class="h-6 w-36 rounded-sm border border-border bg-background px-1 font-mono text-[11px]" />
                </label>
                <Button size="sm" class="h-6 text-[11px]" onclick={applyFrame}>apply</Button>
                <Button size="sm" variant="ghost" class="h-6 text-[11px]" onclick={autoFrame} title="Back to the automatic 30-minute-gap rule">auto</Button>
                <Button size="sm" variant="ghost" class="h-6 text-[11px]" onclick={() => (framing = false)}>cancel</Button>
              </div>
            {/if}
            <p class="text-[12px]">
              {zoneCtx?.current ? displayZoneName(zoneCtx.current) : 'no zone parsed yet'}
            </p>

            <div class="mt-2 grid grid-cols-2 gap-3 border-t border-border/50 pt-2">
              <div>
                <p class="text-[10px] uppercase tracking-wide text-muted-foreground">XP</p>
                {#if $session?.xp_pct_per_hour != null}
                  <p class="text-[13px]">{$session.xp_pct_per_hour.toFixed(2)}%/hr</p>
                {:else}
                  <p class="text-[11px] text-muted-foreground">not enough data yet</p>
                {/if}
                {#if $session?.levels_gained}
                  <p class="text-[11px] text-muted-foreground">+{$session.levels_gained} level{$session.levels_gained === 1 ? '' : 's'} this session</p>
                {/if}
                {#if $session?.eta_hours != null}
                  <p class="text-[11px] text-muted-foreground">{fmtHours($session.eta_hours)} to next level</p>
                {/if}
              </div>
              <div>
                <p class="text-[10px] uppercase tracking-wide text-muted-foreground">Platinum</p>
                {#if $session?.platinum_per_hour != null}
                  <p class="text-[13px]">{Math.round($session.platinum_per_hour).toLocaleString()}pp/hr</p>
                {:else}
                  <p class="text-[11px] text-muted-foreground">not enough data yet</p>
                {/if}
                {#if $session && $session.aa_spent > 0}
                  <p class="text-[11px] text-muted-foreground">{$session.aa_spent} AA spent</p>
                {/if}
              </div>
            </div>

            {#if $session && $session.motes_found > 0}
              <div class="mt-2 border-t border-border/50 pt-2">
                <p class="text-[10px] uppercase tracking-wide text-muted-foreground">
                  Motes
                  {#if $session.motes_per_hour != null}
                    <span class="normal-case text-muted-foreground/70">({Math.round($session.motes_per_hour)}/hr)</span>
                  {/if}
                </p>
                <div class="mt-1 flex flex-wrap gap-3">
                  {#each $session.mote_tiers as t (t.name)}
                    <div class="flex flex-col items-center gap-0.5">
                      <span class="text-[9px] text-muted-foreground">{t.name}</span>
                      <span class="size-4 rounded-full border border-border/50" style="background-color: {TIER_COLORS[t.name] ?? '#8b93a1'}" title={t.name}></span>
                      <span class="text-[10px] text-foreground">{t.count.toLocaleString()}</span>
                    </div>
                  {/each}
                </div>
              </div>
            {/if}

            <button type="button" class="mt-2 text-[11px] text-brand-soft hover:text-primary hover:underline" onclick={() => goto('maps')}>
              open Maps →
            </button>
          </CardContent>
        </Card>
      </div>

      <div class="flex flex-col gap-3">
      <Card class="rounded-sm">
        <CardContent class="px-3 py-2.5">
          <div class="mb-1.5 flex items-center justify-between">
            <h2 class="stat-figure text-[18px]">Loot History</h2>
            <button type="button" class="text-[11px] text-brand-soft hover:text-primary hover:underline" onclick={() => (lootExpanded = true)}>
              expand ⤢
            </button>
          </div>
          {#if !mobs}
            <p class="text-[11px] text-muted-foreground">Loading…</p>
          {:else if !mobs.length}
            <p class="text-[11px] text-muted-foreground">No confirmed pulls yet this session.</p>
          {:else}
            <p class="text-[11px] text-muted-foreground">
              {totalKills.toLocaleString()} kill{totalKills === 1 ? '' : 's'} across {mobs.length.toLocaleString()} mob type{mobs.length === 1
                ? ''
                : 's'} ({totalPulls.toLocaleString()} pull{totalPulls === 1 ? '' : 's'} total)
            </p>
            <ul class="mt-1.5 flex flex-col gap-0.5 text-[11px]">
              {#each topMobs as m (m.name)}
                <li class="flex justify-between">
                  <span class="text-foreground">{m.name}</span>
                  <span class="text-muted-foreground">{m.kills.toLocaleString()} kill{m.kills === 1 ? '' : 's'}</span>
                </li>
              {/each}
            </ul>
          {/if}
        </CardContent>
      </Card>

      <Card class="rounded-sm">
        <CardContent class="px-3 py-2.5">
          <div class="mb-1.5 flex items-center justify-between">
            <h2 class="stat-figure text-[18px]">Group Buffs</h2>
            <!-- why: the mute is one-way from here -- the full line list,
                 muted entries included, lives in the Overlay module -->
            <button type="button" class="text-[11px] text-brand-soft hover:text-primary hover:underline" onclick={() => goto('overlay')}>
              Settings →
            </button>
          </div>
          {#if !buffs}
            <p class="text-[11px] text-muted-foreground">Loading…</p>
          {:else if !buffs.rows.length && !buffs.innates.length}
            <p class="text-[11px] text-muted-foreground">Nothing confirmed yet -- no party classes detected.</p>
          {:else if !neededBuffs.length}
            <p class="text-[11px] text-muted-foreground">Every buff you could have on is on.</p>
          {:else}
            {#snippet buffList(items: typeof neededBuffs, heading: string)}
              <p class="mt-1 text-[10px] text-muted-foreground first:mt-0">{heading}</p>
              <ul class="flex flex-col gap-0.5 text-[11px]">
                {#each items as b (b.line)}
                  <li class="flex items-center justify-between gap-2">
                    <span class="truncate text-foreground">
                      {b.line}
                      <!-- why: an upgradeable row is UP, just from a
                           worse rank -- saying only the better line
                           reads as missing when it is not -->
                      {#if b.was}<span class="text-muted-foreground">(over {b.was})</span>{/if}
                      {#if b.who}<span class="text-muted-foreground">· {b.who}</span>{/if}
                    </span>
                    <span class="flex shrink-0 items-center gap-1.5">
                      <span class="text-muted-foreground">{b.label}</span>
                      <!-- why: the Drop Watch bell's opposite number, and
                           always visible -- reported as "I dont see any
                           bells in that section", which hover-to-reveal
                           earns: an affordance nobody finds is not one. -->
                      <button
                        type="button"
                        class="rounded-sm p-0.5 text-muted-foreground/60 hover:text-bad"
                        title="Stop tracking {b.line} -- undo in Settings -> Overlay -> Group Buffs"
                        onclick={() => void muteLine(b.line)}
                      >
                        <BellOffIcon class="size-3" />
                      </button>
                    </span>
                  </li>
                {/each}
              </ul>
            {/snippet}
            {#if missingOwn.length}
              {@render buffList(missingOwn, 'warning -- your own self-buffs')}
            {/if}
            {#if missingParty.length}
              {@render buffList(missingParty, 'others missing -- expected from your party')}
            {/if}
          {/if}
        </CardContent>
      </Card>
      </div>
    </div>
  {/if}
</div>
