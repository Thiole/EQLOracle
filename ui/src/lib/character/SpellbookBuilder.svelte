<script lang="ts">
  import { Card, CardContent } from '$lib/components/ui/card';
  import { Button } from '$lib/components/ui/button';
  import { Input } from '$lib/components/ui/input';
  import * as Select from '$lib/components/ui/select';
  import { spells, spellEffects, spellStackingGroups } from '$lib/stores/gamedata';
  import { activeClasses, spellRanks, damageSpells } from '$lib/stores/character';
  import { ICON_BASE, ALL_CLASSES, MAX_CHARACTER_LEVEL } from '$lib/character/constants';
  import {
    usableClasses, isUsable, usableByClasses, isBuff, isSoloTarget, isTeamTarget,
    pickBuffSuggestions, pickSupportSuggestions, simulateRotation,
  } from '$lib/character/spellSuggest';
  import DpsSuggest from './DpsSuggest.svelte';
  import { api, type UiFileInfoDto, type ParsedUiFileDto, type SpellDto, type DamageSpellDto } from '$lib/tauri/api';

  // why: a spellbook holds up to 14 spells -- 8 base slots plus up to 6
  // more unlocked by the Mnemonic Retention AA (1 extra slot per AA
  // level, 6 levels total). This is the spellbook itself (which spells
  // are known/slotted), not the hotkey/action bars -- a separate, later
  // concept this doesn't model.
  const BASE_SLOTS = 8;
  const MNEMONIC_RETENTION_LEVELS = 6;
  const DEFAULT_SLOTS = BASE_SLOTS + MNEMONIC_RETENTION_LEVELS;
  const STORAGE_KEY = 'eqlp-spellbook-builder-v2';
  // why: what a dragged spell's own dataTransfer payload is tagged with
  // -- scoped so dropping something dragged in from elsewhere in the OS
  // (a stray text selection, say) can't silently fill a slot.
  const DRAG_MIME = 'application/x-eqlp-spell';

  interface Spellbook {
    name: string;
    slots: (string | null)[];
  }

  function loadBooks(): Spellbook[] {
    try {
      const raw = localStorage.getItem(STORAGE_KEY);
      if (raw) {
        const saved = JSON.parse(raw) as Spellbook[];
        // why: pad any book saved under an older, smaller slot count up
        // to today's default rather than silently dropping/truncating.
        return saved.map((b) => ({ ...b, slots: [...b.slots, ...Array(Math.max(0, DEFAULT_SLOTS - b.slots.length)).fill(null)] }));
      }
    } catch {
      // why: a private window, cleared site data, or a browser blocking
      // storage access all read as "nothing saved yet" -- never a
      // reason to fail the whole page.
    }
    return [{ name: 'Spellbook 1', slots: Array(DEFAULT_SLOTS).fill(null) }];
  }

  let books = $state<Spellbook[]>(loadBooks());

  $effect(() => {
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(books));
    } catch {
      // why: same as loadBooks -- a save that can't land shouldn't throw
      // and break the page; the plan just stays session-only this time.
    }
  });

  // why: display only -- `spellRanks` (and the backend) deal in plain
  // integers, but the game itself always shows a rank as a roman
  // numeral ("Ice Comet X", never "Ice Comet 10"), so the picker matches
  // that instead of introducing a number format the game never uses.
  const ROMAN_TABLE: [number, string][] = [
    [1000, 'M'], [900, 'CM'], [500, 'D'], [400, 'CD'],
    [100, 'C'], [90, 'XC'], [50, 'L'], [40, 'XL'],
    [10, 'X'], [9, 'IX'], [5, 'V'], [4, 'IV'], [1, 'I'],
  ];
  function toRoman(n: number): string {
    let out = '';
    let rem = n;
    for (const [v, sym] of ROMAN_TABLE) {
      while (rem >= v) {
        out += sym;
        rem -= v;
      }
    }
    return out || String(n);
  }

  function addBook() {
    books = [...books, { name: `Spellbook ${books.length + 1}`, slots: Array(DEFAULT_SLOTS).fill(null) }];
  }

  function removeBook(i: number) {
    books = books.filter((_, idx) => idx !== i);
  }

  function renameBook(i: number, name: string) {
    books = books.map((b, idx) => (idx === i ? { ...b, name } : b));
  }

  function setSlot(bookIdx: number, slotIdx: number, name: string | null) {
    books = books.map((b, idx) => (idx === bookIdx ? { ...b, slots: b.slots.map((s, si) => (si === slotIdx ? name : s)) } : b));
  }

  function clearBook(bookIdx: number) {
    books = books.map((b, idx) => (idx === bookIdx ? { ...b, slots: b.slots.map(() => null) } : b));
  }

  // why: fills only empty slots, in order -- never overwrites a manual
  // pick; that's exactly why `clear` exists as its own separate button.
  function fillEmptySlots(bookIdx: number, names: string[]) {
    let i = 0;
    books = books.map((b, idx) => {
      if (idx !== bookIdx) return b;
      return {
        ...b,
        slots: b.slots.map((s) => (s == null && i < names.length ? names[i++] : s)),
      };
    });
  }

  function emptySlotCount(bookIdx: number): number {
    return books[bookIdx]?.slots.filter((s) => s == null).length ?? 0;
  }

  function bookNames(bookIdx: number): string[] {
    return (books[bookIdx]?.slots ?? []).filter((s): s is string => s != null);
  }

  function suggestSoloBuff(bookIdx: number) {
    const count = emptySlotCount(bookIdx);
    if (count <= 0) return;
    const pool = $spells.filter((s) => isUsable(s) && isBuff(s) && isSoloTarget(s));
    const picks = pickBuffSuggestions(pool, $activeClasses, bookNames(bookIdx), count, $spellStackingGroups);
    fillEmptySlots(bookIdx, picks);
  }

  function suggestTeamBuff(bookIdx: number) {
    const count = emptySlotCount(bookIdx);
    if (count <= 0) return;
    const pool = $spells.filter((s) => isUsable(s) && isBuff(s) && isTeamTarget(s));
    const picks = pickBuffSuggestions(pool, $activeClasses, bookNames(bookIdx), count, $spellStackingGroups);
    fillEmptySlots(bookIdx, picks);
  }

  // why: leads with the actual DPS-optimal rotation (real damage math,
  // real weaving), then tops up with best-guess supporting skills
  // (debuffs/CC) since there's no real ranking system for those yet.
  function suggestCombat(bookIdx: number) {
    let count = emptySlotCount(bookIdx);
    if (count <= 0) return;
    const usableDamage = $damageSpells.filter((s) => usableByClasses(s.classes, $activeClasses));
    const { sequence } = simulateRotation(usableDamage, 60);
    const distinct: string[] = [];
    for (const s of sequence) {
      if (!distinct.includes(s.name)) distinct.push(s.name);
    }
    const rotationPicks = distinct.slice(0, count);
    fillEmptySlots(bookIdx, rotationPicks);
    count = emptySlotCount(bookIdx);
    if (count <= 0) return;
    const damageSpellNames = new Set($damageSpells.map((s) => s.name));
    const supportPicks = pickSupportSuggestions(
      $spells, $spellEffects, $activeClasses, bookNames(bookIdx), count, $spellStackingGroups, damageSpellNames,
    );
    fillEmptySlots(bookIdx, supportPicks);
  }

  // why: which slot a click (not a drag) should land in -- drag/drop
  // targets its own drop point directly and never touches this; this is
  // only for the click-a-result fallback, for anyone/anywhere drag
  // doesn't work well.
  let armed = $state<{ book: number; slot: number } | null>(null);

  let search = $state('');

  // ---------------------------------------------------------- suggestions

  // why: "rank10" used to be a raw filter to spells already maxed live;
  // now a hypothetical-max-rank DPS preview instead ("what would be best
  // once maxed", not "what's already maxed") -- see its own tab section below.
  type Mode = 'buffs' | 'combat' | 'rank10';
  let mode = $state<Mode>('buffs');

  // why: defaults to the character's own confirmed 3-class trio, but
  // stays a light toggle -- any class can be added/removed, and an empty
  // selection means "don't filter by class" rather than "show nothing".
  // Only seeded once, the first time activeClasses actually has data, so
  // it doesn't stomp the player's own later edits every time the store
  // ticks.
  let selectedClasses = $state<string[]>([]);
  let classesSeeded = false;
  $effect(() => {
    if (!classesSeeded && $activeClasses.length) {
      selectedClasses = [...$activeClasses];
      classesSeeded = true;
    }
  });

  function toggleClass(c: string) {
    selectedClasses = selectedClasses.includes(c) ? selectedClasses.filter((x) => x !== c) : [...selectedClasses, c];
  }

  const MAX_RANK = 10;

  // why: light default ranking so the picker is useful before typing --
  // in buffs mode, solo/target spells lead (the actual ask); then
  // whether it belongs to one of the player's 3 active classes (a real
  // reported bug: this game has genuine level-60 raid content on
  // classes the player doesn't even play, e.g. Necromancer's
  // `Trucidation`/Bard's `Angstlich's Assonance` -- sorting by raw level
  // *before* known-class buried this character's own level 43-49
  // Wizard nukes, "Ice Comet"/"Conflagration", off the visible list
  // entirely under a pile of other classes' 60s); then highest/most-
  // recent level within that tier, then class name, then spell name. A
  // spell can list several classes at several levels; the known-class
  // entry (if any) drives its level/class here, since that's the one
  // this character would actually see in their own spellbook.
  function sortKey(s: SpellDto): [number, number, number, string, string] {
    const usable = usableClasses(s.classes);
    const known = usable.filter((c) => $activeClasses.includes(c.class));
    const pool = known.length ? known : usable;
    const level = pool.length ? Math.max(...pool.map((c) => c.level ?? 0)) : 0;
    const bestClass = pool.length ? [...pool].sort((a, b) => a.class.localeCompare(b.class))[0].class : '';
    const soloTier = mode === 'buffs' ? (isSoloTarget(s) ? 0 : 1) : 0;
    return [soloTier, known.length ? 0 : 1, -level, bestClass, s.name];
  }

  const ROWS_PER_PAGE = 20;
  const RESULTS_PER_ROW = 8;
  const RESULTS_LIMIT = ROWS_PER_PAGE * RESULTS_PER_ROW;

  // why: one shape for the grid regardless of source DTO -- `badge` is a
  // roman rank for Buffs/Combat, a projected DPS number for the "All
  // rank 10" preview (fetched separately below, only while that tab is open).
  interface DisplayItem {
    name: string;
    icon: string | null;
    badge: string | null;
  }

  let rank10Spells = $state<DamageSpellDto[] | null>(null);
  let rank10Error = $state<string | null>(null);
  $effect(() => {
    if (mode !== 'rank10' || rank10Spells !== null) return;
    api
      .getDamageSpells(true)
      .then((s) => (rank10Spells = s))
      .catch((e) => (rank10Error = e instanceof Error ? e.message : String(e)));
  });

  function fmtDps(n: number): string {
    return n.toLocaleString(undefined, { maximumFractionDigits: 0 });
  }

  const searchResults = $derived.by((): DisplayItem[] => {
    const q = search.trim().toLowerCase();
    if (mode === 'rank10') {
      let pool = (rank10Spells ?? []).filter((s) => isUsable(s));
      if (selectedClasses.length) pool = pool.filter((s) => usableClasses(s.classes).some((c) => selectedClasses.includes(c.class)));
      if (q) pool = pool.filter((s) => s.name.toLowerCase().includes(q));
      return [...pool]
        .sort((a, b) => b.dps_with_reuse - a.dps_with_reuse)
        .slice(0, RESULTS_LIMIT)
        .map((s) => ({ name: s.name, icon: s.icon, badge: `${fmtDps(s.dps_with_reuse)} dps` }));
    }
    let pool = $spells.filter((s) => isUsable(s) && (mode === 'buffs' ? isBuff(s) : !isBuff(s)));
    if (selectedClasses.length) pool = pool.filter((s) => usableClasses(s.classes).some((c) => selectedClasses.includes(c.class)));
    if (q) pool = pool.filter((s) => s.name.toLowerCase().includes(q));
    return [...pool]
      .sort((a, b) => {
        const ka = sortKey(a);
        const kb = sortKey(b);
        for (let i = 0; i < ka.length; i++) {
          if (ka[i] < kb[i]) return -1;
          if (ka[i] > kb[i]) return 1;
        }
        return 0;
      })
      .slice(0, RESULTS_LIMIT)
      .map((s) => ({ name: s.name, icon: s.icon, badge: $spellRanks[s.name] != null ? toRoman($spellRanks[s.name]) : null }));
  });

  function placeInArmedSlot(name: string) {
    if (!armed) return;
    setSlot(armed.book, armed.slot, name);
  }

  function onDragStart(e: DragEvent, name: string) {
    e.dataTransfer?.setData(DRAG_MIME, name);
    e.dataTransfer!.effectAllowed = 'copy';
    // why: without this the browser's default drag preview is a full
    // snapshot of the source element (icon row is a shrunk full-width
    // grid cell, so the preview came out oversized) -- pin the preview to
    // just the icon itself instead, at a small fixed size.
    const icon = (e.currentTarget as HTMLElement).querySelector('img');
    if (icon) e.dataTransfer?.setDragImage(icon, 8, 8);
  }

  function onSlotDrop(e: DragEvent, bookIdx: number, slotIdx: number) {
    e.preventDefault();
    const name = e.dataTransfer?.getData(DRAG_MIME);
    if (name) setSlot(bookIdx, slotIdx, name);
  }

  // ---------------------------------------------------------- UI file import

  let uiFiles = $state<UiFileInfoDto[] | null>(null);
  let uiFilesError = $state<string | null>(null);
  let selectedFile = $state<string>('');
  let parsedFile = $state<ParsedUiFileDto | null>(null);
  let parsedFileError = $state<string | null>(null);

  // why: only the real hotbutton-content files matter here -- the
  // `UI_...` layout files (window position/size, never contents) have
  // nothing to do with spells, so there's no reason to make the player
  // pick past them. The two are linked purely by sharing the same
  // `<Character>_<Zone>` stem with/without the `UI_` prefix -- confirmed
  // directly, not assumed.
  const hotbuttonFiles = $derived(uiFiles?.filter((f) => f.kind === 'hotbuttons') ?? null);

  $effect(() => {
    api
      .listUiFiles()
      .then((f) => (uiFiles = f))
      .catch((e) => (uiFilesError = e instanceof Error ? e.message : String(e)));
  });

  async function loadFile(file: string) {
    selectedFile = file;
    parsedFile = null;
    parsedFileError = null;
    try {
      parsedFile = await api.getUiFile(file);
    } catch (e) {
      parsedFileError = e instanceof Error ? e.message : String(e);
    }
  }

  const hotButtonsSection = $derived(parsedFile?.sections.find((s) => s.name === 'HotButtons') ?? null);

  function fileLabel(f: UiFileInfoDto): string {
    return `${f.character} — ${f.zone}${f.is_backup ? ' (backup)' : ''}`;
  }
</script>

<div class="flex flex-col gap-3">
  <Card class="rounded-sm">
    <CardContent class="px-3 py-2.5 text-[11px] text-muted-foreground">
      Pick spells into named spellbooks (up to 14 slots -- 8 base, plus 6 more as Mnemonic Retention is leveled), purely as a plan for
      now -- this doesn't touch your real game files yet. Which spell sits in which slot is server-tracked character state (never
      written to a local file at all, confirmed against a real dump), so this can't read your *current* spellbook either; it's a place
      to lay out what you want. Writing a finished spellbook back into your own hotbutton file
      (<code class="rounded bg-muted px-1 py-0.5">&lt;Character&gt;_&lt;Zone&gt;_LO1.ini</code>) without disturbing anything else in
      it is planned for later.
    </CardContent>
  </Card>

  {#each books as book, bookIdx (bookIdx)}
    <Card class="rounded-sm">
      <CardContent class="px-3 py-2.5">
        <div class="mb-2 flex flex-wrap items-center gap-2">
          <Input value={book.name} oninput={(e) => renameBook(bookIdx, e.currentTarget.value)} class="h-7 max-w-48 text-[12px]" />
          <Button size="sm" variant="outline" class="h-7 text-[11px]" onclick={() => clearBook(bookIdx)}>clear</Button>
          <Button size="sm" variant="outline" class="h-7 text-[11px]" onclick={() => suggestSoloBuff(bookIdx)}>suggest solo buff</Button>
          <Button size="sm" variant="outline" class="h-7 text-[11px]" onclick={() => suggestTeamBuff(bookIdx)}>suggest team buff</Button>
          <Button size="sm" variant="outline" class="h-7 text-[11px]" onclick={() => suggestCombat(bookIdx)}>suggest combat</Button>
          {#if books.length > 1}
            <Button size="sm" variant="ghost" class="h-7 text-[11px] text-destructive" onclick={() => removeBook(bookIdx)}>remove spellbook</Button>
          {/if}
        </div>
        <div class="grid grid-cols-4 gap-1.5 sm:grid-cols-7">
          {#each book.slots as spellName, slotIdx (slotIdx)}
            {@const isArmed = armed?.book === bookIdx && armed?.slot === slotIdx}
            <div class="flex flex-col gap-0.5">
              <span class="text-[9px] text-muted-foreground">{slotIdx + 1}{#if slotIdx >= BASE_SLOTS}<span title="unlocked by Mnemonic Retention">*</span>{/if}</span>
              {#if spellName}
                <button
                  type="button"
                  class="flex h-10 flex-col items-center justify-center rounded-sm border border-primary/40 bg-primary/10 px-1 text-center text-[10px] text-foreground hover:border-destructive hover:bg-destructive/10 hover:text-destructive"
                  title="click to clear"
                  ondragover={(e) => e.preventDefault()}
                  ondrop={(e) => onSlotDrop(e, bookIdx, slotIdx)}
                  onclick={() => setSlot(bookIdx, slotIdx, null)}
                >
                  {spellName}{#if $spellRanks[spellName] != null}<span class="text-muted-foreground"> ({toRoman($spellRanks[spellName])})</span>{/if}
                </button>
              {:else}
                <button
                  type="button"
                  class="flex h-10 items-center justify-center rounded-sm border border-dashed text-[10px] {isArmed
                    ? 'border-primary text-primary'
                    : 'border-border text-muted-foreground hover:border-primary hover:text-primary'}"
                  ondragover={(e) => e.preventDefault()}
                  ondrop={(e) => onSlotDrop(e, bookIdx, slotIdx)}
                  onclick={() => (armed = { book: bookIdx, slot: slotIdx })}
                >
                  {isArmed ? 'drop here' : 'empty'}
                </button>
              {/if}
            </div>
          {/each}
        </div>
      </CardContent>
    </Card>
  {/each}

  <Button size="sm" variant="outline" class="w-fit" onclick={addBook}>+ add spellbook</Button>

  <!-- why: a big, persistent suggestion block at the bottom -- asked
       directly for this instead of a popup that opens per-slot: drag
       any result straight onto any slot in any spellbook above, no
       need to click a slot first. Clicking a result still works too,
       filling whichever slot was last clicked (`armed`), for drag not
       landing cleanly or just preferring clicks. -->
  <Card class="rounded-sm">
    <CardContent class="px-3 py-2.5">
      <div class="mb-1.5 flex items-center justify-between gap-2">
        <h2 class="panel-title">Suggested spells</h2>
        <Input bind:value={search} placeholder="search spells…" class="h-7 w-56 text-[12px]" />
      </div>

      <!-- why: kept tiny/compact on purpose -- these are filters glanced
           at once and left alone, not a form worth taking up real
           vertical space from the grid below. -->
      <div class="mb-2 flex flex-wrap items-center gap-x-3 gap-y-1.5">
        <div class="flex overflow-hidden rounded-sm border border-border text-[10px]">
          {#each [['buffs', 'Buffs'], ['combat', 'Combat'], ['rank10', `All rank ${MAX_RANK}`]] as [v, label] (v)}
            <button
              type="button"
              class="px-2 py-0.5 {mode === v ? 'bg-primary/15 text-primary' : 'text-muted-foreground hover:bg-accent'}"
              onclick={() => (mode = v as Mode)}
            >
              {label}
            </button>
          {/each}
        </div>
        <div class="grid grid-flow-col grid-rows-4 gap-x-1.5 gap-y-1">
          {#each ALL_CLASSES as c (c)}
            <button
              type="button"
              class="rounded-sm border px-1.5 py-0.5 text-[10px] {selectedClasses.includes(c)
                ? 'border-primary/40 bg-primary/10 text-primary'
                : 'border-border text-muted-foreground hover:border-primary hover:text-primary'}"
              onclick={() => toggleClass(c)}
            >
              {c}
            </button>
          {/each}
        </div>
      </div>

      <p class="mb-2 text-[11px] text-muted-foreground">
        {#if mode === 'rank10'}
          What would be the best damage spells if every one of them were already rank {MAX_RANK} -- a "what's worth
          ranking up" preview, not a filter on what's already maxed. Sorted by projected sustained DPS.
        {:else}
          {#if mode === 'buffs'}Solo/target buffs first, then{/if} your active classes first, highest usable level within (level
          {MAX_CHARACTER_LEVEL} cap -- anything above that isn't learnable yet, so it's left out).
        {/if}
        Drag a result onto any slot above{#if armed}, or click one to fill spellbook "{books[armed.book]?.name}", slot
        {armed.slot + 1}{/if}.
      </p>
      {#if mode === 'rank10' && rank10Error}
        <p class="text-[12px] text-destructive">{rank10Error}</p>
      {:else if mode === 'rank10' && rank10Spells === null}
        <p class="text-[12px] text-muted-foreground">Loading…</p>
      {:else if searchResults.length}
        <div class="grid grid-cols-8 gap-x-2 gap-y-0.5">
          <!-- why: index-keyed, not s.name -- real bug, caught live: spell
               *names* aren't unique across the catalog ("Shield of
               Thorns" is two separate entries), and DamageSpellDto (the
               rank10 source) has no id field to key on instead. This
               list is fully regenerated on every filter/sort change
               anyway, so there's no per-item identity worth preserving. -->
          {#each searchResults as s, i (i)}
            <button
              type="button"
              draggable="true"
              ondragstart={(e) => onDragStart(e, s.name)}
              onclick={() => placeInArmedSlot(s.name)}
              class="flex items-center gap-1 rounded-sm px-1 py-0.5 text-left text-[10px] leading-tight text-foreground hover:bg-accent active:cursor-grabbing"
              title={s.name}
            >
              {#if s.icon}
                <img src={ICON_BASE + encodeURIComponent(s.icon)} alt="" class="size-4 shrink-0 rounded-[2px] border border-border bg-muted/20" />
              {:else}
                <span class="size-4 shrink-0 rounded-[2px] border border-dashed border-border"></span>
              {/if}
              <span class="truncate">{s.name}</span>
              {#if s.badge}<span class="shrink-0 text-muted-foreground">({s.badge})</span>{/if}
            </button>
          {/each}
        </div>
      {:else}
        <p class="text-[11px] text-muted-foreground">no matches</p>
      {/if}
    </CardContent>
  </Card>

  <DpsSuggest />

  <Card class="rounded-sm">
    <CardContent class="px-3 py-2.5">
      <h2 class="panel-title mb-1.5">Your hotbutton files</h2>
      <p class="mb-2 text-[11px] text-muted-foreground">
        Read-only for now -- browse what's actually in your own <code class="rounded bg-muted px-1 py-0.5">&lt;Character&gt;_&lt;Zone&gt;_LO1.ini</code>
        files, the ones that actually hold hotbutton assignments (their `UI_`-prefixed counterparts are window layout only, never
        contents, so they're left out of this list).
      </p>
      {#if uiFilesError}
        <p class="text-[12px] text-destructive">{uiFilesError}</p>
      {:else if !hotbuttonFiles}
        <p class="text-[12px] text-muted-foreground">Loading…</p>
      {:else if !hotbuttonFiles.length}
        <p class="text-[12px] text-muted-foreground">No hotbutton files found in your game folder yet.</p>
      {:else}
        <Select.Root type="single" value={selectedFile} onValueChange={(v) => v && loadFile(v)}>
          <Select.Trigger class="h-7 w-72 text-[12px]">{selectedFile ? fileLabel(hotbuttonFiles.find((f) => f.file === selectedFile)!) : 'choose a file…'}</Select.Trigger>
          <Select.Content>
            {#each hotbuttonFiles as f (f.file)}
              <Select.Item value={f.file}>{fileLabel(f)}</Select.Item>
            {/each}
          </Select.Content>
        </Select.Root>

        {#if parsedFileError}
          <p class="mt-2 text-[12px] text-destructive">{parsedFileError}</p>
        {:else if parsedFile}
          {#if parsedFile.skipped_garbage_lines > 10}
            <p class="mt-2 text-[11px] text-caution">
              Heads up: this file has {parsedFile.skipped_garbage_lines} lines of unrelated text before its first real section. The real
              settings after it still parsed fine.
            </p>
          {/if}
          {#if hotButtonsSection}
            <div class="mt-2 max-h-64 overflow-y-auto">
              <table class="w-full text-[11px]">
                <tbody>
                  {#each hotButtonsSection.entries as [key, value] (key)}
                    <tr class="border-b border-border/50">
                      <td class="px-2 py-0.5 text-muted-foreground">{key}</td>
                      <td class="px-2 py-0.5 font-mono">{value}</td>
                    </tr>
                  {/each}
                </tbody>
              </table>
            </div>
          {:else}
            <p class="mt-2 text-[11px] text-muted-foreground">No [HotButtons] section found -- {parsedFile.sections.length} other section(s) in this file.</p>
          {/if}
        {/if}
      {/if}
    </CardContent>
  </Card>
</div>
