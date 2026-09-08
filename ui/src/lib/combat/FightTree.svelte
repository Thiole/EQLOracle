<script lang="ts">
  // why: the details-style list that IS the scope -- visits as folders,
  // fights under them, ranges at the bottom. Click picks one; ctrl
  // toggles; shift spans; dragging a box selects what it covers.
  import ChevronIcon from '@lucide/svelte/icons/chevron-right';
  import FolderIcon from '@lucide/svelte/icons/folder';
  import CalendarIcon from '@lucide/svelte/icons/calendar';
  import SwordsIcon from '@lucide/svelte/icons/swords';
  import SkullIcon from '@lucide/svelte/icons/skull';
  import ClockIcon from '@lucide/svelte/icons/clock';
  import {
    zoneVisits,
    visitFights,
    expandedVisits,
    expandedDays,
    toggleDayExpanded,
    dayOf,
    encounterMobs,
    expandedEncounters,
    toggleEncounterExpanded,
    selection,
    ranges,
    followCurrentFight,
    visitKey,
    memberKey,
    isSelected,
    toggleVisitExpanded,
    setSelection,
    clearSelection,
    followCurrent,
    addRange,
    removeRange,
    type Member,
  } from '$lib/stores/combat';
  import { fmtDuration } from '$lib/format';
  import { fmtLogTime, logMsToLocalInput, localInputToLogMs } from '$lib/utils';

  // ---------------------------------------------------------------- rows
  type Row = { key: string; member: Member; depth: 0 | 1 | 2 | 3; label: string; detail: string; tag: string; open: boolean };
  const WEEKDAY = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
  const dayLabel = (day: string) => `${WEEKDAY[new Date(`${day}T00:00:00Z`).getUTCDay()]} ${day}`;
  // why: "Hide empty visits" -- a zone you passed through with no fight
  // is noise in a list about fights; the current visit stays so "now"
  // never vanishes. Remembered per browser, on by default
  const HIDE_EMPTY_KEY = 'eqlp.tree.hideEmpty';
  const loadHideEmpty = () => {
    try {
      const raw = localStorage.getItem(HIDE_EMPTY_KEY);
      if (raw !== null) return raw === '1';
    } catch {
      // why: blocked storage -- the default is fine
    }
    return true;
  };
  let hideEmpty = $state(loadHideEmpty());
  $effect(() => {
    try {
      localStorage.setItem(HIDE_EMPTY_KEY, hideEmpty ? '1' : '0');
    } catch {
      // why: nothing to do
    }
  });
  const visits = $derived(hideEmpty ? $zoneVisits.filter((v) => v.fight_count > 0 || v.current) : $zoneVisits);
  const rows = $derived.by((): Row[] => {
    const out: Row[] = [];
    // why: visits arrive newest first; a day opens when its first visit does
    let openDay: string | null = null;
    for (const v of visits) {
      const day = dayOf(v.start_ms);
      if (day !== openDay) {
        openDay = day;
        const inDay = visits.filter((x) => dayOf(x.start_ms) === day);
        out.push({
          key: `d:${day}`,
          member: { kind: 'day', day },
          depth: 0,
          label: dayLabel(day),
          detail: `${inDay.length} · ${inDay.reduce((n, x) => n + x.fight_count, 0)}`,
          tag: inDay.some((x) => x.current) ? 'now' : '',
          open: false,
        });
      }
      if (!$expandedDays.has(day)) continue;
      const k = visitKey(v.index);
      out.push({
        key: `v:${k}`,
        member: { kind: 'visit', visit: v.index },
        depth: 1,
        label: v.label,
        detail: `${v.fight_count}`,
        tag: v.current ? 'now' : '',
        open: false,
      });
      if (!$expandedVisits.has(k)) continue;
      for (const e of $visitFights[k] ?? []) {
        const others = e.entities.length > 1 ? ` +${e.entities.length - 1}` : '';
        out.push({
          key: `e:${e.id}`,
          member: { kind: 'encounter', id: e.id, visit: v.index, target: e.target },
          depth: 2,
          label: `${e.target}${others}`,
          detail: `${fmtDuration(e.duration_ms)} · ${e.total_damage.toLocaleString()}`,
          tag: e.open ? 'live' : e.slain ? 'kill' : e.wiped ? 'wipe' : 'reset',
          open: e.open,
        });
        if (!$expandedEncounters.has(e.id)) continue;
        // why: every enemy in the pull -- what it took, what it dealt
        for (const m of $encounterMobs[e.id] ?? []) {
          out.push({
            key: `m:${e.id}:${m.name}`,
            member: { kind: 'mob', id: e.id, visit: v.index, name: m.name },
            depth: 3,
            label: m.name,
            detail: `${m.damage_taken.toLocaleString()} · ${m.damage_dealt.toLocaleString()}`,
            tag: m.slain ? 'slain' : '',
            open: false,
          });
        }
      }
    }
    for (const r of $ranges) {
      out.push({
        key: `r:${r.since}-${r.until}`,
        member: { kind: 'range', since: r.since, until: r.until },
        depth: 0,
        label: `${fmtLogTime(r.since)} → ${fmtLogTime(r.until)}`,
        detail: fmtDuration(r.until - r.since),
        tag: 'range',
        open: false,
      });
    }
    return out;
  });
  const selectedKeys = $derived(new Set($selection.map(memberKey)));

  // ---------------------------------------------------------------- picking
  let anchor = $state<string | null>(null);
  function pick(row: Row, e: MouseEvent) {
    const cur = $selection;
    if (e.shiftKey && anchor) {
      const a = rows.findIndex((r) => r.key === anchor);
      const b = rows.findIndex((r) => r.key === row.key);
      if (a >= 0 && b >= 0) {
        const [lo, hi] = a < b ? [a, b] : [b, a];
        const span = rows.slice(lo, hi + 1).map((r) => r.member);
        // why: shift extends from the anchor; with ctrl it adds to what is there
        const base = e.ctrlKey || e.metaKey ? cur : [];
        void setSelection(dedupe([...base, ...span]));
        return;
      }
    }
    anchor = row.key;
    if (e.ctrlKey || e.metaKey) {
      void setSelection(isSelected(row.member, cur) ? cur.filter((m) => memberKey(m) !== row.key) : [...cur, row.member]);
      return;
    }
    void setSelection([row.member]);
  }
  function dedupe(list: Member[]): Member[] {
    const seen = new Set<string>();
    return list.filter((m) => {
      const k = memberKey(m);
      if (seen.has(k)) return false;
      seen.add(k);
      return true;
    });
  }

  // ---------------------------------------------------------------- drag box
  // why: the drag is tracked on the window, not by pointer capture --
  // capture retargets the click, and rows would never get theirs. The
  // box is committed once on release: one selection change per drag.
  let list = $state<HTMLDivElement | null>(null);
  let drag = $state<{ x: number; y: number; cx: number; cy: number; additive: boolean; live: boolean } | null>(null);
  let dragHits = $state<Set<string>>(new Set());
  function onPointerDown(e: PointerEvent) {
    if (e.button !== 0 || !list) return;
    if ((e.target as HTMLElement).closest('button')) return;
    drag = { x: e.clientX, y: e.clientY, cx: e.clientX, cy: e.clientY, additive: e.ctrlKey || e.metaKey, live: false };
  }
  function onPointerMove(e: PointerEvent) {
    if (!drag || !list) return;
    drag = { ...drag, cx: e.clientX, cy: e.clientY };
    if (!drag.live && Math.hypot(drag.cx - drag.x, drag.cy - drag.y) < 4) return;
    drag.live = true;
    const box = boxRect();
    const hits = new Set<string>();
    for (const el of list.querySelectorAll<HTMLElement>('[data-row]')) {
      const r = el.getBoundingClientRect();
      if (r.bottom >= box.top && r.top <= box.bottom && r.right >= box.left && r.left <= box.right) hits.add(el.dataset.row!);
    }
    dragHits = hits;
  }
  function onPointerUp() {
    if (!drag) return;
    if (drag.live) {
      const picked = rows.filter((r) => dragHits.has(r.key)).map((r) => r.member);
      const base = drag.additive ? $selection : [];
      void setSelection(dedupe([...base, ...picked]));
    }
    drag = null;
    dragHits = new Set();
  }
  function boxRect() {
    const d = drag!;
    return { left: Math.min(d.x, d.cx), top: Math.min(d.y, d.cy), right: Math.max(d.x, d.cx), bottom: Math.max(d.y, d.cy) };
  }
  const boxStyle = $derived.by(() => {
    if (!drag?.live || !list) return '';
    const b = boxRect();
    const host = list.getBoundingClientRect();
    return `left:${b.left - host.left + list.scrollLeft}px;top:${b.top - host.top + list.scrollTop}px;width:${b.right - b.left}px;height:${b.bottom - b.top}px`;
  });

  // ---------------------------------------------------------------- ranges
  let ranging = $state(false);
  let rangeStart = $state('');
  let rangeEnd = $state('');
  function openRange() {
    // why: seeded from the one selected fight, so a range starts as "this fight, trimmed"
    const one = $selection.length === 1 && $selection[0].kind === 'encounter' ? $selection[0] : null;
    const e = one ? ($visitFights[visitKey(one.visit)] ?? []).find((x) => x.id === one.id) : undefined;
    rangeStart = e ? logMsToLocalInput(e.start_ms) : '';
    rangeEnd = e?.end_ms != null ? logMsToLocalInput(e.end_ms) : '';
    ranging = true;
  }
  function submitRange() {
    const since = rangeStart ? localInputToLogMs(rangeStart) : null;
    const until = rangeEnd ? localInputToLogMs(rangeEnd) : null;
    if (since == null || until == null || until <= since) return;
    void addRange(since, until);
    ranging = false;
  }

  const selectedCount = $derived($selection.length);
</script>

<svelte:window onpointermove={onPointerMove} onpointerup={onPointerUp} onpointercancel={onPointerUp} />

<div class="flex flex-col rounded-sm border border-border bg-card text-[12px]" data-testid="fight-tree">
  <div class="flex items-center gap-2 border-b border-border px-2 py-1.5">
    <span class="panel-title shrink-0">fights</span>
    <span class="min-w-0 truncate text-[11px] text-muted-foreground">{selectedCount ? `${selectedCount} selected` : 'none selected'}</span>
    <span class="ml-auto flex shrink-0 items-center gap-1">
      <label class="flex items-center gap-1 text-[10px] text-muted-foreground" title="Visits with no fights are left out; the current one always shows">
        <input type="checkbox" bind:checked={hideEmpty} /> Hide empty visits
      </label>
      <button
        type="button"
        class="rounded-sm border px-1 py-0.5 text-[10px] {$followCurrentFight ? 'border-primary text-primary' : 'border-border text-muted-foreground hover:text-foreground'}"
        title="Follow the newest fight as it happens; any pick below turns this off"
        onclick={() => void followCurrent()}>● live</button
      >
      <button
        type="button"
        class="rounded-sm border border-border px-1 py-0.5 text-[10px] text-muted-foreground hover:text-foreground"
        title="Add a log-time range -- counts what fell inside it, fight by fight"
        onclick={() => (ranging ? (ranging = false) : openRange())}>range</button
      >
      {#if selectedCount}
        <button
          type="button"
          class="rounded-sm border border-border px-1 py-0.5 text-[10px] text-muted-foreground hover:text-foreground"
          onclick={() => void clearSelection()}>clear</button
        >
      {/if}
    </span>
  </div>
  {#if ranging}
    <div class="flex flex-wrap items-center gap-1 border-b border-border px-2 py-1 text-[11px]">
      <input type="datetime-local" step="1" bind:value={rangeStart} class="h-6 rounded-sm border border-border bg-background px-1" />
      <span class="text-muted-foreground">→</span>
      <input type="datetime-local" step="1" bind:value={rangeEnd} class="h-6 rounded-sm border border-border bg-background px-1" />
      <button type="button" class="rounded-sm border border-border px-1.5 py-0.5 text-muted-foreground hover:text-foreground" onclick={submitRange}>add</button>
      <span class="text-muted-foreground">log time</span>
    </div>
  {/if}
  <!-- why: fifty rows tall, then it scrolls -->
  <div
    bind:this={list}
    class="relative max-h-[1300px] select-none overflow-y-auto"
    role="listbox"
    aria-multiselectable="true"
    tabindex="-1"
    onpointerdown={onPointerDown}
  >
    {#if rows.length === 0}
      <p class="px-2 py-3 text-muted-foreground">No fights parsed yet.</p>
    {/if}
    {#each rows as row (row.key)}
      {@const selected = selectedKeys.has(row.key) || dragHits.has(row.key)}
      <div
        data-row={row.key}
        role="option"
        aria-selected={selected}
        tabindex="0"
        class="flex h-[26px] cursor-pointer items-center gap-1.5 pr-2 outline-none focus-visible:ring-1 focus-visible:ring-ring {selected ? 'bg-primary/20' : 'hover:bg-muted/40'}"
        style="padding-left: {8 + row.depth * 18}px"
        onclick={(e) => pick(row, e)}
        onkeydown={(e) => {
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault();
            pick(row, e as unknown as MouseEvent);
          }
        }}
      >
        {#if row.member.kind === 'day'}
          {@const day = row.member.day}
          <button
            type="button"
            class="rounded-sm p-0.5 text-muted-foreground hover:text-foreground"
            title={$expandedDays.has(day) ? 'Collapse' : 'Expand'}
            onclick={(e) => {
              e.stopPropagation();
              toggleDayExpanded(day);
            }}
          >
            <ChevronIcon class="size-3 transition-transform {$expandedDays.has(day) ? 'rotate-90' : ''}" />
          </button>
          <CalendarIcon class="size-3.5 shrink-0 text-muted-foreground" />
        {:else if row.member.kind === 'visit'}
          <button
            type="button"
            class="rounded-sm p-0.5 text-muted-foreground hover:text-foreground"
            title={$expandedVisits.has(visitKey(row.member.visit)) ? 'Collapse' : 'Expand'}
            onclick={(e) => {
              e.stopPropagation();
              if (row.member.kind === 'visit') void toggleVisitExpanded(row.member.visit);
            }}
          >
            <ChevronIcon class="size-3 transition-transform {$expandedVisits.has(visitKey(row.member.visit)) ? 'rotate-90' : ''}" />
          </button>
          <FolderIcon class="size-3.5 shrink-0 text-muted-foreground" />
        {:else if row.member.kind === 'encounter'}
          {@const eid = row.member.id}
          <button
            type="button"
            class="rounded-sm p-0.5 text-muted-foreground hover:text-foreground"
            title={$expandedEncounters.has(eid) ? 'Collapse' : 'Show the mobs in this fight'}
            onclick={(e) => {
              e.stopPropagation();
              void toggleEncounterExpanded(eid);
            }}
          >
            <ChevronIcon class="size-3 transition-transform {$expandedEncounters.has(eid) ? 'rotate-90' : ''}" />
          </button>
          <SwordsIcon class="size-3.5 shrink-0 {row.open ? 'text-primary' : 'text-muted-foreground'}" />
        {:else if row.member.kind === 'mob'}
          <SkullIcon class="size-3.5 shrink-0 text-muted-foreground" />
        {:else}
          <ClockIcon class="size-3.5 shrink-0 text-caution" />
        {/if}
        <span class="min-w-0 flex-1 truncate">{row.label}</span>
        <span class="shrink-0 tabular-nums text-muted-foreground" title={row.member.kind === 'mob' ? 'damage taken · damage dealt' : row.member.kind === 'day' ? 'visits · fights' : undefined}>{row.detail}</span>
        {#if row.tag}
          <span
            class="w-9 shrink-0 text-right text-[10px] {row.tag === 'live' || row.tag === 'now'
              ? 'text-primary'
              : row.tag === 'kill' || row.tag === 'slain'
                ? 'text-good'
                : row.tag === 'wipe'
                  ? 'text-bad'
                  : 'text-muted-foreground'}">{row.tag}</span
          >
        {/if}
        {#if row.member.kind === 'range'}
          {@const r = row.member}
          <button
            type="button"
            class="text-muted-foreground hover:text-foreground"
            title="Remove this range"
            onclick={(e) => {
              e.stopPropagation();
              void removeRange(r.since, r.until);
            }}>×</button
          >
        {/if}
      </div>
    {/each}
    {#if drag?.live}
      <div class="pointer-events-none absolute border border-primary bg-primary/10" style={boxStyle}></div>
    {/if}
  </div>
</div>
