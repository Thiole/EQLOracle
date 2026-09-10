<script lang="ts">
  // why: the overlay window's own separate Svelte app -- a distinct
  // webview/JS realm from the main window (see overlay-main.ts), so it
  // can't share the main window's stores directly. One shared bundle for
  // every overlay widget: each widget is its own real OS window (see
  // commands::overlay_label's own doc), and this component renders
  // exactly the one widget its own window's label names (via
  // currentOverlayWidget) -- not a container stacking several widgets,
  // that's the whole point of the per-window split.
  import {
    api,
    type LiveMeterDto,
    type SpellCheckDto,
    type StatusEffectsDto,
    type SkillStatusDto,
    type TargetEffectsDto,
    type DropWatchRowDto,
    type SessionDto,
    type GroupBuffsDto,
  } from '$lib/tauri/api';
  import { listen } from '$lib/tauri/invoke';
  import { currentOverlayWidget } from '$lib/tauri/window';
  import { getCurrentWindow, LogicalSize } from '@tauri-apps/api/window';
  import DpsMeterWidget from './DpsMeterWidget.svelte';
  import SkillTrackerWidget from './SkillTrackerWidget.svelte';
  import DropWatchWidget from './DropWatchWidget.svelte';
  import CCTrackerWidget from './CCTrackerWidget.svelte';
  import SessionWidget from './SessionWidget.svelte';
  import GroupBuffsWidget from './GroupBuffsWidget.svelte';
  import { asCcSize, CC_SIZE_WINDOW_DIMS, DEFAULT_CC_SIZE, type CcSize } from './ccSize';
  import { asBuffLayout, BUFF_LAYOUT_WINDOW_DIMS, DEFAULT_BUFF_LAYOUT, type BuffLayout } from './buffLayout';
  import { asDpsLayout, DPS_LAYOUT_WINDOW_DIMS, type DpsLayout } from './dpsLayout';

  const widget = currentOverlayWidget();

  let opacity = $state(0.85);
  // why: the SEPARATE "everything" fade -- see PreferencesDto's own doc
  // on overlay_dps_meter_overall_opacity. 1.0 (fully opaque) by default.
  let overallOpacity = $state(1.0);
  let trackedSkillNames = $state<string[]>([]);
  let trackedTargetEffectNames = $state<string[]>([]);
  let trackedDropNames = $state<string[]>([]);
  let meter = $state<LiveMeterDto | null>(null);
  let spellCheck = $state<SpellCheckDto | null>(null);
  let status = $state<StatusEffectsDto | null>(null);
  let skills = $state<SkillStatusDto[]>([]);
  let targetEffects = $state<TargetEffectsDto | null>(null);
  let dropRows = $state<DropWatchRowDto[]>([]);
  let sessionData = $state<SessionDto | null>(null);
  let groupBuffsData = $state<GroupBuffsDto | null>(null);
  let ccSize = $state<CcSize>(DEFAULT_CC_SIZE);
  let buffLayout = $state<BuffLayout>(DEFAULT_BUFF_LAYOUT);
  let rootEl: HTMLDivElement | undefined = $state();
  let stageEl = $state<HTMLDivElement | null>(null);
  // why: the size the window opened at is the layout's base -- the
  // Rust side opens every widget at its preset dims, and a preset
  // change re-bases below when it calls setSize. Nothing persists a
  // dragged size, so a fresh open is always the base.
  let baseW = $state(0);
  let baseH = $state(0);
  let scale = $state(1);
  const SCALE_MIN = 0.7;
  const SCALE_MAX = 2.5;
  function rescale() {
    if (!stageEl || !baseW) return;
    const winW = window.innerWidth;
    const winH = window.innerHeight;
    // why: the stage's own unscaled height -- offsetHeight is layout
    // size, unaffected by the transform
    const contentH = stageEl.offsetHeight || baseH || 1;
    const s = Math.min(winW / baseW, winH / contentH);
    scale = Math.max(SCALE_MIN, Math.min(SCALE_MAX, s));
  }
  function rebase(w: number, h: number) {
    baseW = w;
    baseH = h;
    rescale();
  }
  $effect(() => {
    if (!stageEl) return;
    if (!baseW) rebase(window.innerWidth, window.innerHeight);
    const ro = new ResizeObserver(() => rescale());
    ro.observe(stageEl);
    window.addEventListener('resize', rescale);
    rescale();
    return () => {
      ro.disconnect();
      window.removeEventListener('resize', rescale);
    };
  });

  async function refreshPrefs() {
    const p = await api.getPreferences();
    // why: overlay theme should match the main window. Same
    // attribute-on-<html> mechanism app.css's themed pages use (see
    // overlay.css's doc for the themes.css import) -- this window is a
    // separate JS realm, can't share stores/settings.ts's
    // theme.subscribe, applies it independently. Re-applied every
    // refreshPrefs() poll (cheap, no-op if unchanged), so a theme
    // switch in Settings takes effect without reopening this window.
    if (typeof document !== 'undefined') {
      document.documentElement.dataset.theme = p.theme;
    }
    if (widget === 'dps_meter') {
      opacity = p.overlay_dps_meter_opacity;
      overallOpacity = p.overlay_dps_meter_overall_opacity;
    } else if (widget === 'skill_tracker') {
      opacity = p.overlay_skill_tracker_opacity;
      overallOpacity = p.overlay_skill_tracker_overall_opacity;
      // why: re-read every tick, not just on mount -- picking a
      // different skill to track in Settings while this window is
      // already open should show up without needing to reopen it, and
      // this list changes rarely enough that re-fetching preferences
      // alongside the data poll is cheap either way
      trackedSkillNames = p.tracked_skills;
      trackedTargetEffectNames = p.tracked_target_effects;
    } else if (widget === 'drop_watch') {
      opacity = p.overlay_drop_watch_opacity;
      overallOpacity = p.overlay_drop_watch_overall_opacity;
      trackedDropNames = p.tracked_drop_items;
    } else if (widget === 'session') {
      opacity = p.overlay_session_opacity;
      overallOpacity = p.overlay_session_overall_opacity;
    } else if (widget === 'group_buffs') {
      opacity = p.overlay_group_buffs_opacity;
      overallOpacity = p.overlay_group_buffs_overall_opacity;
      // why: render only -- the window's own dims are set at open time
      // and live-resized by the 'overlay-size' listener, same split as
      // ccSize below
      buffLayout = asBuffLayout(p.overlay_group_buffs_layout);
    } else if (widget === 'cc_tracker') {
      opacity = p.overlay_cc_tracker_opacity;
      overallOpacity = p.overlay_cc_tracker_overall_opacity;
      // why: NOT resized here -- this only sets the local class/render
      // size. The window's own dimensions are set once at open time by
      // set_overlay_enabled (reading this same persisted value), and
      // live-resized only by the 'overlay-size' listener below, so a
      // plain poll never fights a mid-drag/mid-resize window.
      ccSize = asCcSize(p.overlay_cc_tracker_size);
    }
  }

  async function refresh() {
    // why: one rejected call used to abandon the whole assignment and
    // leave every value at its last reading, silently, because the
    // callers are all `void refresh()`. Settled per call instead, so a
    // failure costs that one field for one tick, never the display.
    if (widget === 'dps_meter') {
      const [m, sc] = await Promise.allSettled([api.getLiveMeter(), api.getSpellCheck()]);
      if (m.status === 'fulfilled') meter = m.value;
      if (sc.status === 'fulfilled') spellCheck = sc.value;
    } else if (widget === 'skill_tracker') {
      const [s, sk, te, sc] = await Promise.all([
        api.getStatusEffects(),
        api.getSkillStatus(),
        api.getTargetEffects(),
        api.getSpellCheck(),
      ]);
      status = s;
      skills = sk;
      targetEffects = te;
      spellCheck = sc;
    } else if (widget === 'drop_watch') {
      dropRows = await api.getDropWatch();
    } else if (widget === 'session') {
      sessionData = await api.getSession();
    } else if (widget === 'group_buffs') {
      groupBuffsData = await api.getGroupBuffs();
    } else if (widget === 'cc_tracker') {
      status = await api.getStatusEffects();
    }
  }

  // why: every widget window refreshes on every tick, and each refresh is
  // one to four IPC calls that take the ingest lock -- six widgets open
  // during a replay is a steady stream of them against the very lock the
  // parser holds. Nothing they would show mid-replay is worth reading, so
  // they sit still and refresh once when it settles. Same reasoning as
  // tauri/events.ts's own backfill guard.
  let wasBackfilling = false;
  $effect(() => {
    void refreshPrefs();
    void refresh();
    const unlistenTick = listen<{ status?: { backfilling?: boolean } }>('parse-tick', (e) => {
      const backfilling = e.payload?.status?.backfilling ?? false;
      if (!backfilling || wasBackfilling !== backfilling) {
        void refreshPrefs();
        void refresh();
      }
      wasBackfilling = backfilling;
    });
    // why: the tick is the only thing that refreshed this window, so one
    // missed event -- or one rejected refresh, which assigns nothing and
    // keeps the previous value -- froze a live number on screen while the
    // fight carried on. Reported as "the ui had me showing as 21.6k
    // damage for a long time, despite being in combat the rest of the
    // fight": the store and the meter both tracked that fight correctly,
    // the widget just stopped being told. A slow floor, not a poll: the
    // tick still does the real work whenever it arrives.
    const heartbeat = setInterval(() => void refresh(), 2000);
    // why: [widget, value] tuples, NOT bare values -- real bug, caught
    // live: emit_to does not actually scope delivery to one window here.
    // Every overlay-* window shares one capability entry
    // (capabilities/default.json's "overlay-*" glob), and confirmed via
    // temporary two-sided logging, emit_to's permission check treats
    // that whole glob as its audience -- every open overlay window's
    // listener fires on every emit_to call targeting any one of them,
    // regardless of the label actually passed in. So every payload here
    // carries the target widget too, and each window filters to its own
    // identity (the `widget` this component was constructed with) before
    // acting on it -- correct regardless of emit_to's actual scoping,
    // not dependent on trusting it. See commands::set_overlay_opacity's
    // own doc for the Rust side.
    const unlistenOpacity = listen<[string, number]>('overlay-opacity', (e) => {
      if (e.payload[0] !== widget) return;
      opacity = e.payload[1];
    });
    const unlistenOverallOpacity = listen<[string, number]>('overlay-overall-opacity', (e) => {
      if (e.payload[0] !== widget) return;
      overallOpacity = e.payload[1];
    });
    // why: the one live-push that resizes the real OS window, not just a
    // CSS value -- set_overlay_size (commands.rs) only emits, this
    // window is the one that knows its own new dims (see ccSize.ts's own
    // doc) and calls setSize on itself. Only ever emitted for
    // 'cc_tracker' today (see overlay_label's own doc), but filtered by
    // payload widget same as every other event here now -- see the
    // opacity listeners' own doc on why that check can't be skipped even
    // when only one widget currently uses an event.
    const unlistenSize = listen<[string, string]>('overlay-size', (e) => {
      if (e.payload[0] !== widget) return;
      // why: the payload is that widget's own preset -- a size for the CC
      // Tracker, a layout for Group Buffs. Both resize this window.
      if (widget === 'group_buffs') {
        buffLayout = asBuffLayout(e.payload[1]);
        const { w, h } = BUFF_LAYOUT_WINDOW_DIMS[buffLayout];
        rebase(w, h);
        void getCurrentWindow().setSize(new LogicalSize(w, h));
        return;
      }
      if (widget === 'dps_meter') {
        const { w, h } = DPS_LAYOUT_WINDOW_DIMS[asDpsLayout(e.payload[1])];
        rebase(w, h);
        void getCurrentWindow().setSize(new LogicalSize(w, h));
        return;
      }
      ccSize = asCcSize(e.payload[1]);
      const { w, h } = CC_SIZE_WINDOW_DIMS[ccSize];
      rebase(w, h);
      void getCurrentWindow().setSize(new LogicalSize(w, h));
    });
    // why: "where did that window go" -- see commands::locate_overlay's
    // own doc. Toggles the class directly on the real DOM node, NOT via
    // `locating` state -- real bug, caught live: a `locating = false`
    // then `= true` round trip (with a reflow read in between) works on
    // raw DOM but not through Svelte 5 state, since $state writes are
    // batched onto a microtask rather than applied to the DOM
    // synchronously. Reading rootEl.offsetWidth right after the `false`
    // write usually ran before Svelte had actually removed the class,
    // so the two writes collapsed into one net update and the class
    // never left the DOM -- the first-ever flash worked (a genuine
    // absent-to-present transition), every flash after that was a
    // silent no-op (CSS doesn't restart a still-applied animation just
    // because the class re-applies without an intervening reflow, and
    // there wasn't one). classList.remove/offsetWidth/classList.add
    // here are real synchronous DOM calls, no framework batching to
    // fight.
    const unlistenLocate = listen<string>('overlay-locate', (e) => {
      if (e.payload !== widget || !rootEl) return;
      rootEl.classList.remove('locate-flash');
      void rootEl.offsetWidth;
      rootEl.classList.add('locate-flash');
    });
    return () => {
      clearInterval(heartbeat);
      void unlistenTick.then((f) => f());
      void unlistenOpacity.then((f) => f());
      void unlistenOverallOpacity.then((f) => f());
      void unlistenSize.then((f) => f());
      void unlistenLocate.then((f) => f());
    };
  });
</script>

<!-- why: NOT data-tauri-drag-region -- a real check against this exact
     stack (XWayland via KWin) found that move request silently doesn't
     move the window (a resize-border drag does). set_overlay_locked
     switches to real decorations instead while unlocked, so dragging
     the actual title bar (every window manager supports that) repositions it. -->
<!-- why: dynamic scaling -- "instead of having static sizes ... it just
     scales the current layout to the size (with minimums) ... never show
     scroll bars". The widget renders once at its base width (the size
     the window opened at, or the preset it was last resized to) and the
     stage is transform-scaled to fit whatever the window is now: the
     smaller of width-fit and height-fit, so more rows shrink in place
     rather than pushing anything off, and a dragged window fits exactly.
     Clamped to a floor (a 10px row never goes under 7px) and a ceiling;
     past the floor the stage is clipped, never scrolled. transform, not
     zoom: WebKitGTK is the floor. -->
<div bind:this={rootEl} class="h-screen w-screen overflow-hidden">
  <div
    bind:this={stageEl}
    class="p-2"
    style="width: {baseW}px; transform: scale({scale}); transform-origin: top left;"
  >
  {#if widget === 'dps_meter'}
    <!-- why: the landing-average check lives in the Skill Tracker only --
         "you're still showing the x% of usual in dps meter. it shouldnt be
         there. that info is fine in the skill tracker" -->
    <DpsMeterWidget {meter} {opacity} {overallOpacity} />
  {:else if widget === 'skill_tracker'}
    <SkillTrackerWidget
      {status}
      {skills}
      {trackedSkillNames}
      {trackedTargetEffectNames}
      {targetEffects}
      {spellCheck}
      {opacity}
      {overallOpacity}
    />
  {:else if widget === 'drop_watch'}
    <DropWatchWidget rows={dropRows} trackedNames={trackedDropNames} {opacity} {overallOpacity} />
  {:else if widget === 'session'}
    <SessionWidget session={sessionData} {opacity} {overallOpacity} />
  {:else if widget === 'group_buffs'}
    <!-- why: buffs are a between-fights checklist -- "during combat, hide
         the lines ... it should collapse in combat and be 100% hidden".
         An open encounter IS combat (LiveMeterDto.open). -->
    <GroupBuffsWidget data={groupBuffsData} inCombat={meter?.open ?? false} layout={buffLayout} {opacity} {overallOpacity} />
  {:else if widget === 'cc_tracker'}
    <CCTrackerWidget {status} {opacity} {overallOpacity} size={ccSize} />
  {/if}
  </div>
</div>

<style>
  /* why: the one place a scrollbar could still come from -- the document
     itself. The stage is clipped by its parent; nothing scrolls. */
  :global(html),
  :global(body) {
    overflow: hidden;
  }

  /* why: "make it very visible" -- a full-color invert, not a border or
     a tint, so it reads at a glance regardless of the widget's own
     theme/opacity. Same hard on/off house style as every other blink in
     this app (StatusEffectsWidget's status-blink, SkillTrackerWidget's
     target-effect-blink): steps(1, end), a fixed iteration count so it
     settles back to normal on its own rather than flashing forever. On
     transparent: invert only ever touches drawn pixels -- the window's
     own transparent background stays transparent through it. Applied
     via classList directly on rootEl (see the 'overlay-locate'
     listener's own doc), not a template class binding -- :global so it
     doesn't rely on Svelte's own scoping hash still being present on
     the element, and so the compiler doesn't flag it as an unused
     selector for a class it can't see being applied. */
  :global(.locate-flash) {
    animation: locate-flash-anim 0.3s steps(1, end) 8;
  }
  @keyframes locate-flash-anim {
    50% {
      filter: invert(1);
    }
  }
</style>
