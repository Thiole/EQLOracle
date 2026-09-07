<script lang="ts">
  // why: each overlay widget is its own self-contained card -- enable +
  // opacity together, not one shared window-wide toggle/slider, and its
  // own real OS window (see commands::overlay_label's own doc), not
  // content stacked inside one shared overlay surface -- so reposition/
  // lock is per-widget too, not one button for everything. More widgets
  // land as more cards here, each independently on/off, see-through, and
  // positioned.
  import { Card, CardContent } from '$lib/components/ui/card';
  import { Checkbox } from '$lib/components/ui/checkbox';
  import { api } from '$lib/tauri/api';
  import {
    overlayEnabled,
    dpsMeterEnabled,
    dpsMeterOpacity,
    setDpsMeterEnabled,
    setDpsMeterOpacity,
    dpsMeterOverallOpacity,
    setDpsMeterOverallOpacity,
    skillTrackerEnabled,
    skillTrackerOpacity,
    setSkillTrackerEnabled,
    setSkillTrackerOpacity,
    skillTrackerOverallOpacity,
    setSkillTrackerOverallOpacity,
    trackedSkills,
    toggleTrackedSkill,
    trackedTargetEffects,
    toggleTrackedTargetEffect,
    dropWatchEnabled,
    dropWatchOpacity,
    setDropWatchEnabled,
    setDropWatchOpacity,
    dropWatchOverallOpacity,
    setDropWatchOverallOpacity,
    trackedDropItems,
    toggleTrackedDropItem,
    ccTrackerEnabled,
    ccTrackerOpacity,
    setCcTrackerEnabled,
    setCcTrackerOpacity,
    ccTrackerOverallOpacity,
    setCcTrackerOverallOpacity,
    ccTrackerSize,
    setCcTrackerSize,
    sessionWidgetEnabled,
    sessionWidgetOpacity,
    setSessionWidgetEnabled,
    setSessionWidgetOpacity,
    sessionWidgetOverallOpacity,
    setSessionWidgetOverallOpacity,
    groupBuffsEnabled,
    groupBuffsOpacity,
    setGroupBuffsEnabled,
    setGroupBuffsOpacity,
    groupBuffsOverallOpacity,
    setGroupBuffsOverallOpacity,
    mutedBuffLines,
    toggleMutedBuffLine,
    groupBuffsLayout,
    setGroupBuffsLayout,
    dpsMeterLayout,
    setDpsMeterLayout,
    loadPreferences,
  } from '$lib/stores/settings';
  import type { CcSize } from './ccSize';
  import type { BuffLayout } from './buffLayout';
  import { HelpTip } from '$lib/components/ui/help';
  import type { DpsLayout } from './dpsLayout';
  import { windowCapability, loadWindowCapability } from '$lib/stores/overlay';
  import TrackedSkillsList from './TrackedSkillsList.svelte';
  import BellIcon from '@lucide/svelte/icons/bell';
  import BellOffIcon from '@lucide/svelte/icons/bell-off';

  $effect(() => {
    void loadPreferences();
    void loadWindowCapability();
  });

  // why: the mute list needs NAMES, and the only place they exist is the
  // tracker's own catalog -- read once here rather than duplicating the
  // line-grouping rules on this side. Failure is silent and empty: the
  // rest of the card still works with no party detected yet.
  let buffLines = $state<string[]>([]);
  $effect(() => {
    void api
      .getGroupBuffs()
      .then((d) => (buffLines = d?.catalog ?? []))
      .catch(() => (buffLines = []));
  });
  // why: a muted line drops out of the tracker, so the catalog stops
  // naming it -- union with the muted list or a mute could never be undone
  const mutableLines = $derived([...new Set([...buffLines, ...$mutedBuffLines])].sort());

  let enableError = $state<string | null>(null);
  let skillTrackerError = $state<string | null>(null);
  let dropWatchError = $state<string | null>(null);
  let ccTrackerError = $state<string | null>(null);
  let sessionError = $state<string | null>(null);
  let groupBuffsError = $state<string | null>(null);
  // why: each widget's own window starts locked (click-through) --
  // matches every widget window's own real default at open
  let locked = $state<Record<string, boolean>>({
    dps_meter: true,
    skill_tracker: true,
    drop_watch: true,
    cc_tracker: true,
    session: true,
    group_buffs: true,
  });

  async function onToggleDpsMeter(on: boolean) {
    enableError = null;
    try {
      await setDpsMeterEnabled(on);
      locked.dps_meter = true;
    } catch (e) {
      enableError = e instanceof Error ? e.message : String(e);
    }
  }

  async function onToggleSkillTracker(on: boolean) {
    skillTrackerError = null;
    try {
      await setSkillTrackerEnabled(on);
      locked.skill_tracker = true;
    } catch (e) {
      skillTrackerError = e instanceof Error ? e.message : String(e);
    }
  }

  async function onToggleDropWatch(on: boolean) {
    dropWatchError = null;
    try {
      await setDropWatchEnabled(on);
      locked.drop_watch = true;
    } catch (e) {
      dropWatchError = e instanceof Error ? e.message : String(e);
    }
  }

  async function onToggleCcTracker(on: boolean) {
    ccTrackerError = null;
    try {
      await setCcTrackerEnabled(on);
      locked.cc_tracker = true;
    } catch (e) {
      ccTrackerError = e instanceof Error ? e.message : String(e);
    }
  }

  async function onToggleGroupBuffs(on: boolean) {
    groupBuffsError = null;
    try {
      await setGroupBuffsEnabled(on);
      locked.group_buffs = true;
    } catch (e) {
      groupBuffsError = e instanceof Error ? e.message : String(e);
    }
  }

  async function onToggleSession(on: boolean) {
    sessionError = null;
    try {
      await setSessionWidgetEnabled(on);
      locked.session = true;
    } catch (e) {
      sessionError = e instanceof Error ? e.message : String(e);
    }
  }

  // why: reads the widget id off the clicked element's own data-widget
  // attribute, NOT a closure over repositionButton's own `widget`
  // parameter -- real bug, caught live: with this snippet rendered 4
  // times (one per widget Card), a click was firing with a stale/wrong
  // id (always whichever widget was most recently enabled, not the
  // card actually clicked). See OverlayQuickMenu.svelte's own doc on
  // the same fix -- a data-* attribute is tied 1:1 to that one rendered
  // node, there's no closure to go stale.
  function widgetOf(e: Event): string {
    return (e.currentTarget as HTMLElement).dataset.widget ?? '';
  }

  async function toggleLocked(widget: string) {
    locked[widget] = !locked[widget];
    await api.setOverlayLocked(widget, locked[widget]).catch(() => {});
  }

  const capped = $derived($windowCapability?.capability === 'docked');

  // why: single "enable ui" toggle for everything at once, since each
  // widget reopens wherever it was last positioned (see
  // preferences::OverlayPosition's doc). Checked only when EVERY widget
  // is on ("select all", not "any"); clicking always sets every widget
  // to the same new state. Not its own persisted preference -- stays a
  // live, explicit action each session (see preferences.rs's doc on
  // why enabled/disabled stays live-only).
  const allEnabled = $derived(
    $dpsMeterEnabled && $skillTrackerEnabled && $dropWatchEnabled && $ccTrackerEnabled && $sessionWidgetEnabled,
  );
  async function onToggleAll(on: boolean) {
    // why: keeps overlayEnabled (settings.ts) in sync with this page's
    // own "enable ui" action too -- OverlayQuickMenu's top-bar shortcut
    // reads that same flag, so enabling everything from here shouldn't
    // leave the top-bar button/menu still reading "off". Per-widget
    // errors still surface on THIS page individually (see each
    // onToggleX above) -- this just adds the one extra flag set.
    overlayEnabled.set(on);
    await Promise.all([
      onToggleDpsMeter(on),
      onToggleSkillTracker(on),
      onToggleDropWatch(on),
      onToggleCcTracker(on),
      onToggleSession(on),
    ]);
  }
</script>

{#snippet presetPicker(options: string[], current: string, onPick: (v: string) => void, disabled: boolean)}
  <div class="mt-2 flex gap-1">
    {#each options as v (v)}
      <button
        type="button"
        {disabled}
        onclick={() => onPick(v)}
        class="rounded-md border px-2 py-1 text-[11px] {current === v
          ? 'border-primary text-foreground'
          : 'border-border text-muted-foreground hover:border-foreground/30 hover:text-foreground'} {disabled ? 'opacity-40' : ''}"
      >
        {v}
      </button>
    {/each}
  </div>
{/snippet}

{#snippet repositionButton(widget: string)}
  <div class="mt-2 flex items-center gap-2">
    <button
      type="button"
      data-widget={widget}
      class="rounded-md border border-border px-2 py-1 text-[11px] text-muted-foreground hover:border-foreground/30 hover:text-foreground"
      onclick={(e) => void toggleLocked(widgetOf(e))}
    >
      {locked[widget] ? 'unlock to reposition' : 'lock (click-through) — drag its title bar to move it, then lock'}
    </button>
    <button
      type="button"
      data-widget={widget}
      class="rounded-md border border-border px-2 py-1 text-[11px] text-muted-foreground hover:border-foreground/30 hover:text-foreground"
      onclick={(e) => void api.locateOverlay(widgetOf(e))}
      title="Bring this widget's window to front and flash it"
    >
      locate
    </button>
  </div>
{/snippet}

<!-- why: two independent opacity sliders per widget, same snippet
     reused for both, differing only in label/description and preview
     swatch style: "background" only fades the panel (text/icons stay
     fully readable), "everything" is a CSS opacity on the whole widget
     -- text and icons fade with it too. -->
<!-- why: title left, the section's own explanation behind a "?" right.
     Every card uses this, so the page reads as controls and the words
     are one click away rather than permanently in the way. -->
{#snippet sectionHeader(title: string, help: string)}
  <div class="mb-1.5 flex items-start justify-between gap-2">
    <h2 class="panel-title">{title}</h2>
    <HelpTip label="About {title}" text={help} />
  </div>
{/snippet}

{#snippet alphaPreview(
  opacity: number,
  onInput: (v: number) => void,
  disabled: boolean,
  label: string,
  description: string,
  fadesText: boolean,
)}
  <div class="mt-2.5 flex items-center gap-1">
    <p class="text-[11px] text-muted-foreground">{label}</p>
    <HelpTip {label} text={description} />
  </div>
  <div class="mt-1 flex items-center gap-3 {disabled ? 'opacity-40' : ''}">
    <input
      type="range"
      min="0.1"
      max="1"
      step="0.05"
      value={opacity}
      {disabled}
      oninput={(e) => onInput(+e.currentTarget.value)}
      class="h-1.5 max-w-64 flex-1 accent-primary"
    />
    <span class="w-10 shrink-0 text-right text-[12px] tabular-nums text-foreground">{Math.round(opacity * 100)}%</span>
    <!-- why: a real alpha-preview checker, not just a number -- lets you see
         how see-through it'll actually read before it's on screen. The
         "everything" version previews on real sample text, since that's
         the whole point of that slider -- the "background" version keeps
         text out of its own swatch on purpose, since that opacity never touches it. -->
    <div
      class="flex h-8 w-16 shrink-0 items-center justify-center rounded-sm border border-border"
      style="background-image: repeating-conic-gradient(#3a3d42 0% 25%, #26282c 0% 50%); background-size: 8px 8px;"
    >
      <div
        class="flex size-full items-center justify-center rounded-[3px]"
        style:background-color="color-mix(in srgb, var(--background) {(fadesText ? 1 : opacity) * 100}%, transparent)"
        style:opacity={fadesText ? opacity : 1}
      >
        {#if fadesText}
          <span class="text-[9px] font-medium text-foreground">abc</span>
        {/if}
      </div>
    </div>
  </div>
{/snippet}

<div class="flex flex-col gap-3 p-3">
  <Card class="rounded-sm">
    <CardContent class="px-3 py-2.5">
      {@render sectionHeader(
        'overlay',
        "Each widget below is its own little window -- its own on/off, its own transparency, and its own position. \"enable ui\" turns your set back on together; each reopens right where you left it, and position is remembered per widget once you've dragged and locked it in.",
      )}
      {#if !$windowCapability}
        <p class="text-[11px] text-muted-foreground">Checking what this session can do…</p>
      {:else if capped}
        <p class="text-[11px] text-caution">{$windowCapability.reason}</p>
        <p class="mt-1 text-[11px] text-muted-foreground">
          The floating overlay isn't available here -- everything below stays saved for whenever it is.
        </p>
      {:else}
        <label class="mt-2 flex items-center gap-2 text-[12px] text-foreground">
          <Checkbox checked={allEnabled} onCheckedChange={(v: boolean) => void onToggleAll(v)} />
          enable ui
        </label>
      {/if}
    </CardContent>
  </Card>

  <!-- why: max 2 wide, same layout rule as every other module's own
       menu (see Character's own doc) -- these two are the compact
       glance-able ones (a checkbox, a couple sliders, no per-item list),
       so a 50/50 split reads better than either stacking full-width or
       flowing loose with the wider list-heavy cards below. Skill Tracker/
       Drop Watch keep full width -- their own tracked-item lists need
       the room. -->
  <div class="grid grid-cols-2 gap-3">
    <Card class="rounded-sm">
      <CardContent class="px-3 py-2.5">
        {@render sectionHeader(
          'DPS meter',
          "Players and assumed pets, rolling recent-fight damage. Layout: minimal is teammates only; condensed adds the top enemy and one combined row for the rest; full gives every enemy its own row. Pets always fold into one row.",
        )}
        <!-- why: at the top of the section, like Group Buffs -- this is
             how the IN-GAME widget lays itself out, nothing in the app
             changes with it. Allies read the same in all three; what
             changes is the enemy side. -->
        {@render presetPicker(['minimal', 'condensed', 'full'], $dpsMeterLayout, (v) => void setDpsMeterLayout(v as DpsLayout), capped)}
        <div class="mt-2"></div>
        <label class="flex items-center gap-2 text-[12px] {capped ? 'text-muted-foreground' : 'text-foreground'}">
          <Checkbox checked={$dpsMeterEnabled} disabled={capped} onCheckedChange={(v: boolean) => void onToggleDpsMeter(v)} />
          enable
        </label>
        {#if capped}
          <p class="mt-1 text-[11px] text-muted-foreground">Needs the floating overlay -- see above.</p>
        {/if}
        {#if enableError}
          <p class="mt-1 text-[11px] text-bad">{enableError}</p>
        {/if}
        {#if $dpsMeterEnabled && !capped}
          {@render repositionButton('dps_meter')}
        {/if}

        {@render alphaPreview(
          $dpsMeterOpacity,
          (v) => void setDpsMeterOpacity(v),
          capped,
          'background opacity',
          'How see-through the panel behind everything reads -- text and numbers stay fully readable no matter how low this goes.',
          false,
        )}
        {@render alphaPreview(
          $dpsMeterOverallOpacity,
          (v) => void setDpsMeterOverallOpacity(v),
          capped,
          'everything',
          'Fades the whole widget together -- text and numbers included, not just the panel behind them.',
          true,
        )}
      </CardContent>
    </Card>

    <Card class="rounded-sm">
      <CardContent class="px-3 py-2.5">
        {@render sectionHeader(
          'CC tracker',
          "Root / Stun / Fear, three small squares -- lit up when one's on you. Size sets how big the squares draw and resizes the window to match.",
        )}
        <label class="flex items-center gap-2 text-[12px] {capped ? 'text-muted-foreground' : 'text-foreground'}">
          <Checkbox checked={$ccTrackerEnabled} disabled={capped} onCheckedChange={(v: boolean) => void onToggleCcTracker(v)} />
          enable
        </label>
        {#if capped}
          <p class="mt-1 text-[11px] text-muted-foreground">Needs the floating overlay -- see above.</p>
        {/if}
        {#if ccTrackerError}
          <p class="mt-1 text-[11px] text-bad">{ccTrackerError}</p>
        {/if}
        <p class="mt-2 text-[11px] text-muted-foreground">size</p>
        {@render presetPicker(['small', 'medium', 'large'], $ccTrackerSize, (v) => void setCcTrackerSize(v as CcSize), capped)}
        {#if $ccTrackerEnabled && !capped}
          {@render repositionButton('cc_tracker')}
        {/if}

        {@render alphaPreview(
          $ccTrackerOpacity,
          (v) => void setCcTrackerOpacity(v),
          capped,
          'background opacity',
          'How see-through the panel behind the squares reads.',
          false,
        )}
        {@render alphaPreview(
          $ccTrackerOverallOpacity,
          (v) => void setCcTrackerOverallOpacity(v),
          capped,
          'everything',
          'Fades the whole widget together -- the squares included, not just the panel behind them.',
          true,
        )}
      </CardContent>
    </Card>

    <Card class="rounded-sm">
      <CardContent class="px-3 py-2.5">
        {@render sectionHeader(
          'Session',
          "AA, levels, and plat per hour, plus motes found by tier -- this session's own rates.",
        )}
        <label class="flex items-center gap-2 text-[12px] {capped ? 'text-muted-foreground' : 'text-foreground'}">
          <Checkbox checked={$sessionWidgetEnabled} disabled={capped} onCheckedChange={(v: boolean) => void onToggleSession(v)} />
          enable
        </label>
        {#if capped}
          <p class="mt-1 text-[11px] text-muted-foreground">Needs the floating overlay -- see above.</p>
        {/if}
        {#if sessionError}
          <p class="mt-1 text-[11px] text-bad">{sessionError}</p>
        {/if}
        {#if $sessionWidgetEnabled && !capped}
          {@render repositionButton('session')}
        {/if}

        {@render alphaPreview(
          $sessionWidgetOpacity,
          (v) => void setSessionWidgetOpacity(v),
          capped,
          'background opacity',
          'How see-through the panel behind the numbers reads.',
          false,
        )}
        {@render alphaPreview(
          $sessionWidgetOverallOpacity,
          (v) => void setSessionWidgetOverallOpacity(v),
          capped,
          'everything',
          'Fades the whole widget together -- numbers included, not just the panel behind them.',
          true,
        )}
      </CardContent>
    </Card>

    <Card class="rounded-sm">
      <CardContent class="px-3 py-2.5">
        {@render sectionHeader(
          'Group Buffs',
          '"Good" when every buff your party\'s confirmed classes can put on you, that helps your own classes, is on you -- else what\'s missing and who could cast it. Layout: minimal is one verdict line in game, in a window shrunk to fit it.',
        )}
        <!-- why: at the top of the section, above the enable toggle --
             this is how the IN-GAME widget lays itself out, nothing here
             or anywhere else in the app changes with it. -->
        {@render presetPicker(['full', 'minimal'], $groupBuffsLayout, (v) => void setGroupBuffsLayout(v as BuffLayout), capped)}
        <div class="mt-2"></div>
        <label class="flex items-center gap-2 text-[12px] {capped ? 'text-muted-foreground' : 'text-foreground'}">
          <Checkbox checked={$groupBuffsEnabled} disabled={capped} onCheckedChange={(v: boolean) => void onToggleGroupBuffs(v)} />
          enable
        </label>
        <!-- why: the overlay itself is click-through, so the mute lives
             here rather than on the widget -- one row per spell line the
             tracker knows, muted ones kept listed so a mute can be
             undone after the line leaves the catalog. -->
        {#if mutableLines.length}
          <div class="mt-2">
            <div class="mb-1 flex items-center gap-1">
              <span class="text-[11px] text-muted-foreground">spell lines</span>
              <HelpTip
                label="About muting spell lines"
                text="A muted line is never watched and never counted missing, so it cannot hold back the all-clear. Muting is per line and covers every rank of it."
              />
            </div>
            <div role="listbox" aria-label="Group buff lines" class="max-h-40 overflow-y-auto rounded-sm border border-border">
              {#each mutableLines as line (line)}
                {@const muted = $mutedBuffLines.includes(line)}
                <button
                  type="button"
                  role="option"
                  aria-selected={muted}
                  class="flex w-full items-center justify-between gap-2 border-b border-border/50 px-2 py-1 text-left text-[12px] last:border-b-0 {muted
                    ? 'text-muted-foreground line-through'
                    : 'text-foreground hover:bg-muted/40'}"
                  title={muted ? `Watch ${line} again` : `Stop watching ${line}`}
                  onclick={() => void toggleMutedBuffLine(line)}
                >
                  <span class="truncate">{line}</span>
                  {#if muted}
                    <BellOffIcon class="size-3 shrink-0" />
                  {:else}
                    <BellIcon class="size-3 shrink-0 text-muted-foreground" />
                  {/if}
                </button>
              {/each}
            </div>
          </div>
        {/if}
        {#if capped}
          <p class="mt-1 text-[11px] text-muted-foreground">Needs the floating overlay -- see above.</p>
        {/if}
        {#if groupBuffsError}
          <p class="mt-1 text-[11px] text-bad">{groupBuffsError}</p>
        {/if}
        {#if $groupBuffsEnabled && !capped}
          {@render repositionButton('group_buffs')}
        {/if}
        {@render alphaPreview(
          $groupBuffsOpacity,
          (v) => void setGroupBuffsOpacity(v),
          capped,
          'background opacity',
          'How see-through the panel behind the list reads.',
          false,
        )}
        {@render alphaPreview(
          $groupBuffsOverallOpacity,
          (v) => void setGroupBuffsOverallOpacity(v),
          capped,
          'everything',
          'Fades the whole widget together.',
          true,
        )}
      </CardContent>
    </Card>
  </div>

  <Card class="rounded-sm">
    <CardContent class="px-3 py-2.5">
      {@render sectionHeader(
        'skill tracker',
        'Charm, invisibility, hide, and sneak always show; cooldowns below are yours to pick.',
      )}
      <label class="flex items-center gap-2 text-[12px] {capped ? 'text-muted-foreground' : 'text-foreground'}">
        <Checkbox checked={$skillTrackerEnabled} disabled={capped} onCheckedChange={(v: boolean) => void onToggleSkillTracker(v)} />
        enable
      </label>
      {#if capped}
        <p class="mt-1 text-[11px] text-muted-foreground">Needs the floating overlay -- see above.</p>
      {/if}
      {#if skillTrackerError}
        <p class="mt-1 text-[11px] text-bad">{skillTrackerError}</p>
      {/if}
      {#if $skillTrackerEnabled && !capped}
        {@render repositionButton('skill_tracker')}
      {/if}

      <div class="mt-2.5">
        <div class="flex items-center gap-1">
          <p class="text-[11px] text-muted-foreground">tracked cooldowns</p>
          <HelpTip
            label="About tracked cooldowns"
            text="Add an ability from Combat's own breakdown, or track a spell right here."
          />
        </div>
        <div class="mt-1">
          <TrackedSkillsList
            items={$trackedSkills}
            onRemove={(name) => void toggleTrackedSkill(name)}
            ariaLabel="Tracked cooldowns"
            emptyLabel="Nothing tracked yet."
          />
        </div>
      </div>
      <div class="mt-2.5">
        <div class="flex items-center gap-1">
          <p class="text-[11px] text-muted-foreground">target effects</p>
          <HelpTip
            label="About target effects"
            text={'A DoT or debuff -- landed? how long\'s left? Add spells from Character \u2192 Spellbook\'s own "overlay spell tracking" section.'}
          />
        </div>
        <div class="mt-1">
          <TrackedSkillsList
            items={$trackedTargetEffects}
            onRemove={(name) => void toggleTrackedTargetEffect(name)}
            ariaLabel="Tracked target effects"
            emptyLabel="Nothing tracked yet."
          />
        </div>
      </div>

      {@render alphaPreview(
        $skillTrackerOpacity,
        (v) => void setSkillTrackerOpacity(v),
        capped,
        'background opacity',
        'How see-through the panel behind everything reads -- text and icons stay fully readable no matter how low this goes.',
        false,
      )}
      {@render alphaPreview(
        $skillTrackerOverallOpacity,
        (v) => void setSkillTrackerOverallOpacity(v),
        capped,
        'everything',
        'Fades the whole widget together -- text and icons included, not just the panel behind them.',
        true,
      )}
    </CardContent>
  </Card>

  <Card class="rounded-sm">
    <CardContent class="px-3 py-2.5">
      {@render sectionHeader(
        'drop watch',
        'A heads-up when you are fighting a mob known to drop something you are tracking. Add items from Sky Quests and Primary Class Unlocks; nothing is tracked by default.',
      )}
      <label class="flex items-center gap-2 text-[12px] {capped ? 'text-muted-foreground' : 'text-foreground'}">
        <Checkbox checked={$dropWatchEnabled} disabled={capped} onCheckedChange={(v: boolean) => void onToggleDropWatch(v)} />
        enable
      </label>
      {#if capped}
        <p class="mt-1 text-[11px] text-muted-foreground">Needs the floating overlay -- see above.</p>
      {/if}
      {#if dropWatchError}
        <p class="mt-1 text-[11px] text-bad">{dropWatchError}</p>
      {/if}
      {#if $dropWatchEnabled && !capped}
        {@render repositionButton('drop_watch')}
      {/if}

      <div class="mt-2.5">
        <p class="text-[11px] text-muted-foreground">
          tracked drops <span class="text-muted-foreground/70">(add an item from Sky Quests or Primary Class Unlocks)</span>
        </p>
        <div class="mt-1">
          <TrackedSkillsList
            items={$trackedDropItems}
            onRemove={(name) => void toggleTrackedDropItem(name)}
            ariaLabel="Tracked drops"
            emptyLabel="Nothing tracked yet."
          />
        </div>
      </div>

      {@render alphaPreview(
        $dropWatchOpacity,
        (v) => void setDropWatchOpacity(v),
        capped,
        'background opacity',
        'How see-through the panel behind everything reads -- text stays fully readable no matter how low this goes.',
        false,
      )}
      {@render alphaPreview(
        $dropWatchOverallOpacity,
        (v) => void setDropWatchOverallOpacity(v),
        capped,
        'everything',
        'Fades the whole widget together -- text included, not just the panel behind it.',
        true,
      )}
    </CardContent>
  </Card>
</div>
