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
  let rootEl: HTMLDivElement | undefined = $state();

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
    if (widget === 'dps_meter') {
      [meter, spellCheck] = await Promise.all([api.getLiveMeter(), api.getSpellCheck()]);
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

  $effect(() => {
    void refreshPrefs();
    void refresh();
    const unlistenTick = listen('parse-tick', () => {
      void refreshPrefs();
      void refresh();
    });
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
      ccSize = asCcSize(e.payload[1]);
      const { w, h } = CC_SIZE_WINDOW_DIMS[ccSize];
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
<div bind:this={rootEl} class="min-h-screen w-screen p-2">
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
    <GroupBuffsWidget data={groupBuffsData} {opacity} {overallOpacity} />
  {:else if widget === 'cc_tracker'}
    <CCTrackerWidget {status} {opacity} {overallOpacity} size={ccSize} />
  {/if}
</div>

<style>
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
