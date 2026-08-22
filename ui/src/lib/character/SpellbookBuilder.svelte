<script lang="ts">
  import { Card, CardContent } from '$lib/components/ui/card';
  import { Button } from '$lib/components/ui/button';
  import { Input } from '$lib/components/ui/input';
  import * as Select from '$lib/components/ui/select';
  import { spells } from '$lib/stores/gamedata';
  import { activeClasses, spellRanks } from '$lib/stores/character';
  import { ICON_BASE, ALL_CLASSES, MAX_CHARACTER_LEVEL } from '$lib/character/constants';
  import DpsSuggest from './DpsSuggest.svelte';
  import { api, type UiFileInfoDto, type ParsedUiFileDto, type SpellDto } from '$lib/tauri/api';

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

  // why: which slot a click (not a drag) should land in -- drag/drop
  // targets its own drop point directly and never touches this; this is
  // only for the click-a-result fallback, for anyone/anywhere drag
  // doesn't work well.
  let armed = $state<{ book: number; slot: number } | null>(null);

  let search = $state('');

  // ---------------------------------------------------------- suggestions

  // why: a spell's own `spell_type` is the wiki's finest-grained real
  // category (checked against actual data: 1,928 spells, ~20 distinct
  // values) -- these are the beneficial-flavored ones. Everything else
  // (Detrimental, Direct Damage, DoT, Slow, Curse, Pet, and anything
  // unrecognised) falls into "combat" by exclusion, so every spell lands
  // in exactly one of the two buckets, never neither.
  const BENEFICIAL_TYPES = new Set([
    'Beneficial', 'Statistic Buff', 'Resist Buff', 'Utility Beneficial', 'Heal', 'Heal Over Time',
    'Pet Buff', 'Pet Heal', 'Haste', 'Cure', 'Movement Buff', 'Remove Curse',
  ]);
  function isBuff(s: SpellDto): boolean {
    return !!s.spell_type && BENEFICIAL_TYPES.has(s.spell_type);
  }

  // why: "solo/target" = it lands on you or one other friendly, not a
  // whole group -- exactly the set worth prioritizing for a spellbook
  // that isn't assuming a full group is always up.
  const SOLO_TARGET_TYPES = new Set(['Self', 'Single', 'Single Friendly (or Self)']);
  function isSoloTarget(s: SpellDto): boolean {
    return !!s.target_type && SOLO_TARGET_TYPES.has(s.target_type);
  }

  // why: real bug, reported directly -- the data has genuine level 51-60
  // entries (later/raid-tier content), but this game's actual character
  // level cap is `MAX_CHARACTER_LEVEL` (50, same number `setLevel`
  // already clamps to), so nobody can learn those yet no matter how
  // "high level" they look. A class entry over the cap doesn't make a
  // spell usable for that class -- filtered out entirely below rather
  // than just deprioritized, since a currently-unlearnable spell isn't
  // a real suggestion.
  function usableClasses(s: SpellDto) {
    return s.classes.filter((c) => c.level == null || c.level <= MAX_CHARACTER_LEVEL);
  }
  function isUsable(s: SpellDto): boolean {
    return usableClasses(s).length > 0;
  }

  // why: "rank10" is a third, orthogonal view, not a buff/combat split
  // -- everything this character has actually maxed out live, across
  // every spell type, so a player scanning for "what's left to upgrade"
  // can see what's already done and rule it out at a glance.
  type Mode = 'buffs' | 'combat' | 'rank10';
  let mode = $state<Mode>('buffs');
  const MAX_RANK = 10;

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
    const usable = usableClasses(s);
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

  const searchResults = $derived.by(() => {
    const q = search.trim().toLowerCase();
    let pool = $spells.filter((s) => {
      if (!isUsable(s)) return false;
      if (mode === 'rank10') return $spellRanks[s.name] === MAX_RANK;
      return mode === 'buffs' ? isBuff(s) : !isBuff(s);
    });
    if (selectedClasses.length) pool = pool.filter((s) => usableClasses(s).some((c) => selectedClasses.includes(c.class)));
    if (q) pool = pool.filter((s) => s.name.toLowerCase().includes(q));
    return [...pool].sort((a, b) => {
      const ka = sortKey(a);
      const kb = sortKey(b);
      for (let i = 0; i < ka.length; i++) {
        if (ka[i] < kb[i]) return -1;
        if (ka[i] > kb[i]) return 1;
      }
      return 0;
    }).slice(0, RESULTS_LIMIT);
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
        <div class="mb-2 flex items-center gap-2">
          <Input value={book.name} oninput={(e) => renameBook(bookIdx, e.currentTarget.value)} class="h-7 max-w-48 text-[12px]" />
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
          Every spell (any type) you've already ranked to {MAX_RANK} this session, across your selected class(es) -- so it's
          obvious what's already maxed and what's still worth spending motes on. Rank is only known once you've cast a spell
          this session, so one you ranked up earlier but haven't cast yet won't show up here until you do.
        {:else}
          {#if mode === 'buffs'}Solo/target buffs first, then{/if} your active classes first, highest usable level within (level
          {MAX_CHARACTER_LEVEL} cap -- anything above that isn't learnable yet, so it's left out).
        {/if}
        Drag a result onto any slot above{#if armed}, or click one to fill spellbook "{books[armed.book]?.name}", slot
        {armed.slot + 1}{/if}.
      </p>
      {#if searchResults.length}
        <div class="grid grid-cols-8 gap-x-2 gap-y-0.5">
          {#each searchResults as s (s.id)}
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
              {#if $spellRanks[s.name] != null}<span class="shrink-0 text-muted-foreground">({toRoman($spellRanks[s.name])})</span>{/if}
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
