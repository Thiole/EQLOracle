<script lang="ts">
  import { Card, CardContent } from '$lib/components/ui/card';
  import { Button } from '$lib/components/ui/button';
  import { Input } from '$lib/components/ui/input';
  import * as Select from '$lib/components/ui/select';
  import { spells, spellEffects, spellStackingGroups } from '$lib/stores/gamedata';
  import { activeClasses, spellRanks, damageSpells } from '$lib/stores/character';
  import { spellLineOverrides, spellLineCustomMembership, openSpellLinePriority } from '$lib/stores/spellLinePriority';
  import { ICON_BASE, ALL_CLASSES, MAX_CHARACTER_LEVEL } from '$lib/character/constants';
  import {
    usableClasses, isUsable, usableByClasses, isBuff, isSoloTarget, isTeamTarget,
    pickBuffSuggestions, pickSupportSuggestions, simulateRotation, resistTypeOf,
  } from '$lib/character/spellSuggest';
  import DpsSuggest from './DpsSuggest.svelte';
  import { api, type UiFileInfoDto, type SpellDto, type DamageSpellDto, type SpellbookFileDto } from '$lib/tauri/api';
  import { status } from '$lib/stores/status';

  // why: a real loadout holds up to 14 spells -- 8 base slots plus up to
  // 6 more unlocked by the Mnemonic Retention AA (1 extra slot per AA
  // level, 6 levels total) -- matches spellbookfiles.rs's own MAX_SLOTS.
  // what a dragged spell's own dataTransfer payload is tagged with --
  // scoped so dropping something dragged in from elsewhere in the OS (a
  // stray text selection, say) can't silently fill a slot.
  const DRAG_MIME = 'application/x-eqlp-spell';

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

  // why: which slot a click (not a drag) should land in -- drag/drop
  // targets its own drop point directly and never touches this; this is
  // only for the click-a-result fallback, for anyone drag doesn't work
  // well for. The name still needs resolving to a real numeric id
  // before it can land in a slot -- see placeInArmedSlot.
  type Armed = { loadoutIndex: number; slot: number };
  let armed = $state<Armed | null>(null);

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

  async function placeInArmedSlot(name: string) {
    if (!armed) return;
    await placeInLoadoutSlot(armed.loadoutIndex, armed.slot, name);
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

  function onLoadoutSlotDrop(e: DragEvent, loadoutIndex: number, slot: number) {
    e.preventDefault();
    const name = e.dataTransfer?.getData(DRAG_MIME);
    if (name) void placeInLoadoutSlot(loadoutIndex, slot, name);
  }

  // ---------------------------------------------------------- UI file import

  let uiFiles = $state<UiFileInfoDto[] | null>(null);
  let uiFilesError = $state<string | null>(null);
  let selectedFile = $state<string>('');

  // why: only the real hotbutton-content files matter here -- the
  // `UI_...` layout files (window position/size, never contents) have
  // nothing to do with spells, so there's no reason to make the player
  // pick past them. The two are linked purely by sharing the same
  // `<Character>_<Zone>` stem with/without the `UI_` prefix -- confirmed
  // directly, not assumed. Sorted so whichever file matches the
  // currently-tailed log's own character/server (real evidence, the
  // same identity tail_worker.rs already resolves) sorts first --
  // almost always the one you actually want, without hunting for it.
  const hotbuttonFiles = $derived.by(() => {
    const files = uiFiles?.filter((f) => f.kind === 'hotbuttons') ?? null;
    if (!files) return null;
    const char = $status?.status.character;
    const server = $status?.status.server;
    const matches = (f: UiFileInfoDto) => char != null && server != null && f.character === char && f.zone === server;
    return [...files].sort((a, b) => Number(matches(b)) - Number(matches(a)));
  });

  // why: lands on the suggested file automatically instead of making
  // the player pick it themselves -- only once, and only while nothing's
  // been picked yet, so it never yanks a deliberate manual pick away
  let suggestedFileLoaded = false;
  $effect(() => {
    if (suggestedFileLoaded || !hotbuttonFiles?.length || selectedFile) return;
    const char = $status?.status.character;
    const server = $status?.status.server;
    const suggested = hotbuttonFiles.find((f) => f.character === char && f.zone === server);
    if (suggested) {
      suggestedFileLoaded = true;
      void loadFile(suggested.file);
    }
  });

  $effect(() => {
    api
      .listUiFiles()
      .then((f) => (uiFiles = f))
      .catch((e) => (uiFilesError = e instanceof Error ? e.message : String(e)));
  });

  function fileLabel(f: UiFileInfoDto): string {
    const char = $status?.status.character;
    const server = $status?.status.server;
    const suggested = f.character === char && f.zone === server ? ' (current)' : '';
    return `${f.character} — ${f.zone}${suggested}${f.is_backup ? ' (backup)' : ''}`;
  }

  // ---------------------------------------------------------- real spellbook loadouts
  //
  // why: the game's own [SpellLoadouts] section, in the same file
  // fileLabel/loadFile pick above -- real, confirmed-by-reading-the-
  // actual-files data (spellbookfiles.rs's own doc has the numbers).
  // Loaded once per file pick, edited in place (Svelte 5's own deep
  // $state reactivity), saved back as the full 60-entry shape the
  // backend expects -- nothing here is a plan; every edit is a real
  // pending change to a real game file until "save" actually writes it.

  let spellbookFile = $state<SpellbookFileDto | null>(null);
  let spellbookLoadError = $state<string | null>(null);
  let loadoutActionError = $state<string | null>(null);
  let saving = $state(false);
  let savedAt = $state<Date | null>(null);

  // why: collapsed by default -- a decent-sized window should show every
  // section's header at a glance, not one already expanded to full height.
  let foundOpen = $state(false);
  let suggestedOpen = $state(false);

  // why: "save as" forks the current file pair under a new name instead
  // of overwriting it -- a small inline box (not a full dialog, nothing
  // else here needs one) for the one field it actually needs.
  let showSaveAsBox = $state(false);
  let newStemInput = $state('');
  let savingAs = $state(false);
  let saveAsError = $state<string | null>(null);

  function openSaveAsBox() {
    newStemInput = '';
    saveAsError = null;
    showSaveAsBox = true;
  }

  async function saveLoadoutsAsNewFile() {
    if (!spellbookFile) return;
    const stem = newStemInput.trim();
    if (!stem) {
      saveAsError = 'Name it Character_Zone, matching the game\'s own file naming.';
      return;
    }
    savingAs = true;
    saveAsError = null;
    try {
      const newFile = await api.saveSpellbookFileAs(spellbookFile.file, stem, spellbookFile.loadouts);
      uiFiles = await api.listUiFiles();
      showSaveAsBox = false;
      await loadFile(newFile);
    } catch (e) {
      saveAsError = e instanceof Error ? e.message : String(e);
    } finally {
      savingAs = false;
    }
  }

  // why: every real loadout the file has, all loaded and shown at once
  // (not one picked at a time) -- direct correction: "load them all at
  // once, as the successive spellbooks, so when I save to file, it
  // saves it all into the same UI file, the same way it is parsed."
  // save_spellbook_file already always writes the whole 60-entry shape
  // back regardless; this just makes the UI match that -- edit any of
  // them inline, one save commits the lot.
  const inUseLoadouts = $derived(spellbookFile?.loadouts.filter((l) => l.in_use) ?? []);

  function loadoutByIndex(index: number) {
    return spellbookFile?.loadouts.find((l) => l.index === index) ?? null;
  }

  async function loadFile(file: string) {
    selectedFile = file;
    spellbookFile = null;
    spellbookLoadError = null;
    savedAt = null;
    try {
      spellbookFile = await api.loadSpellbookFile(file);
    } catch (e) {
      spellbookLoadError = e instanceof Error ? e.message : String(e);
    }
  }

  async function placeInLoadoutSlot(loadoutIndex: number, slot: number, name: string) {
    const lo = loadoutByIndex(loadoutIndex);
    if (!lo) return;
    loadoutActionError = null;
    const ids = await api.resolveSpellbookSpellIds([name]).catch((e) => {
      loadoutActionError = e instanceof Error ? e.message : String(e);
      return null;
    });
    const id = ids?.[0];
    if (id == null) {
      loadoutActionError ??= `"${name}" has no matching in-game spell id, from this install's own spells_us.txt -- can't place it in a real slot.`;
      return;
    }
    const s = lo.slots.find((s) => s.slot === slot);
    if (s) {
      s.spell_id = id;
      s.name = name;
      s.catalog_id = null;
    }
  }

  function clearLoadoutSlot(loadoutIndex: number, slot: number) {
    const s = loadoutByIndex(loadoutIndex)?.slots.find((s) => s.slot === slot);
    if (s) {
      s.spell_id = -1;
      s.name = null;
      s.catalog_id = null;
    }
  }

  function renameLoadout(loadoutIndex: number, name: string) {
    const lo = loadoutByIndex(loadoutIndex);
    if (lo) lo.name = name;
  }

  function loadoutNames(lo: SpellbookFileDto['loadouts'][number]): string[] {
    return lo.slots.filter((s) => s.name != null).map((s) => s.name as string);
  }

  function loadoutEmptySlotCount(lo: SpellbookFileDto['loadouts'][number]): number {
    return lo.slots.filter((s) => s.name == null).length;
  }

  // why: real, one-click "start over" for a loadout -- pairs with the
  // suggest buttons below, which only ever fill empty slots (never
  // overwrite a manual pick); clear-then-suggest is the easy overwrite.
  function clearLoadout(loadoutIndex: number) {
    const lo = loadoutByIndex(loadoutIndex);
    if (!lo) return;
    for (const s of lo.slots) clearLoadoutSlot(loadoutIndex, s.slot);
  }

  // why: one batched id-resolution round trip for the whole fill, not
  // one per slot -- real, measured fix: up to 14 sequential
  // resolveSpellbookSpellIds calls each reread and reparsed all of
  // spells_us.txt (73,971 lines) from scratch, the "reallllly slow"
  // case reported live. First N empty slots in slot order get
  // names[0..N], same order the old one-at-a-time loop filled them in.
  async function fillLoadoutEmptySlots(loadoutIndex: number, names: string[]) {
    const lo = loadoutByIndex(loadoutIndex);
    if (!lo || !names.length) return;
    const targets = lo.slots.filter((s) => s.name == null).slice(0, names.length);
    if (!targets.length) return;
    const picks = names.slice(0, targets.length);
    loadoutActionError = null;
    const ids = await api.resolveSpellbookSpellIds(picks).catch((e) => {
      loadoutActionError = e instanceof Error ? e.message : String(e);
      return null;
    });
    if (!ids) return;
    const unresolved: string[] = [];
    targets.forEach((s, i) => {
      const id = ids[i];
      if (id == null) {
        unresolved.push(picks[i]);
        return;
      }
      s.spell_id = id;
      s.name = picks[i];
      s.catalog_id = null;
    });
    if (unresolved.length) {
      loadoutActionError = `${unresolved.length} suggested spell${unresolved.length > 1 ? 's' : ''} had no matching in-game id, from this install's own spells_us.txt -- skipped: ${unresolved.join(', ')}`;
    }
  }

  // why: these three mirror the old virtual-book suggest buttons, now
  // applied directly to a real loadout instead of a separate local-only
  // planning copy -- filtered by whichever classes are toggled in
  // Suggested spells below (selectedClasses), same as its own search
  // results, not the character's fixed active-class trio.
  async function suggestLoadoutSoloBuff(loadoutIndex: number) {
    const lo = loadoutByIndex(loadoutIndex);
    if (!lo) return;
    const count = loadoutEmptySlotCount(lo);
    if (count <= 0) return;
    const pool = $spells.filter((s) => isUsable(s) && isBuff(s) && isSoloTarget(s));
    const picks = pickBuffSuggestions(
      pool, selectedClasses, loadoutNames(lo), count, $spellStackingGroups, $spellLineOverrides, $spellLineCustomMembership,
    );
    await fillLoadoutEmptySlots(loadoutIndex, picks);
  }

  async function suggestLoadoutTeamBuff(loadoutIndex: number) {
    const lo = loadoutByIndex(loadoutIndex);
    if (!lo) return;
    const count = loadoutEmptySlotCount(lo);
    if (count <= 0) return;
    const pool = $spells.filter((s) => isUsable(s) && isBuff(s) && isTeamTarget(s));
    const picks = pickBuffSuggestions(
      pool, selectedClasses, loadoutNames(lo), count, $spellStackingGroups, $spellLineOverrides, $spellLineCustomMembership,
    );
    await fillLoadoutEmptySlots(loadoutIndex, picks);
  }

  async function suggestLoadoutCombat(loadoutIndex: number) {
    const lo = loadoutByIndex(loadoutIndex);
    if (!lo) return;
    let count = loadoutEmptySlotCount(lo);
    if (count <= 0) return;
    const usableDamage = $damageSpells.filter((s) => usableByClasses(s.classes, selectedClasses));
    const { sequence } = simulateRotation(usableDamage, 60);
    const distinct: string[] = [];
    for (const s of sequence) {
      if (!distinct.includes(s.name)) distinct.push(s.name);
    }
    await fillLoadoutEmptySlots(loadoutIndex, distinct.slice(0, count));
    count = loadoutEmptySlotCount(lo);
    if (count <= 0) return;
    const damageSpellNames = new Set($damageSpells.map((s) => s.name));
    const rotationResistTypes = new Set(
      usableDamage.map((s) => resistTypeOf(s.resist)).filter((t): t is string => t != null),
    );
    const supportPicks = pickSupportSuggestions(
      $spells, $spellEffects, selectedClasses, loadoutNames(lo), count, $spellStackingGroups, damageSpellNames,
      $spellLineOverrides, $spellLineCustomMembership, rotationResistTypes,
    );
    await fillLoadoutEmptySlots(loadoutIndex, supportPicks);
  }

  function addNewLoadout() {
    if (!spellbookFile) return;
    loadoutActionError = null;
    const free = spellbookFile.loadouts.find((l) => !l.in_use);
    if (!free) {
      loadoutActionError = 'All 60 real loadout slots this game reserves are already in use.';
      return;
    }
    free.in_use = true;
    free.name = `New Loadout ${free.index}`;
    free.slots = Array.from({ length: 14 }, (_, i) => ({ slot: i + 1, spell_id: -1, name: null, catalog_id: null }));
  }

  function deleteLoadout(index: number) {
    const lo = loadoutByIndex(index);
    if (!lo) return;
    if (!confirm(`Delete loadout "${lo.name}"? A backup of the file as it was before saving is kept alongside it either way.`)) return;
    lo.in_use = false;
    lo.name = null;
    lo.slots = [];
  }

  async function saveLoadouts() {
    if (!spellbookFile) return;
    saving = true;
    loadoutActionError = null;
    savedAt = null;
    try {
      await api.saveSpellbookFile(spellbookFile.file, spellbookFile.loadouts);
      savedAt = new Date();
    } catch (e) {
      loadoutActionError = e instanceof Error ? e.message : String(e);
    } finally {
      saving = false;
    }
  }
</script>

<div class="flex flex-col gap-3">
  <Card class="rounded-sm">
    <CardContent class="px-3 py-2.5 text-[11px] font-medium text-destructive">
      AUTO suggests are work in progress, please triple check. The auto rules are currently: Optimal DPS Loop + best cc or shot term
      buff spells for remaining available combat lines it tries its best to not have multiple from same spell line. I am working on
      specific overrides like Mez which is level 16 for example as "outliers"
    </CardContent>
  </Card>

  <Card class="rounded-sm">
    <CardContent class="px-3 py-2.5">
      <button type="button" class="flex w-full items-center gap-1.5 text-left" onclick={() => (foundOpen = !foundOpen)}>
        <span class="w-6 text-[26px] leading-none font-bold text-foreground">{foundOpen ? '▾' : '▸'}</span>
        <h2 class="panel-title">Found spellbooks</h2>
      </button>
      {#if foundOpen}
      <p class="mb-2 mt-1.5 text-[11px] text-muted-foreground">
        Reads, edits, and writes your game's own named spellbook-loadout presets -- a real, client-side quick-swap feature saved in your
        <code class="rounded bg-muted px-1 py-0.5">&lt;Character&gt;_&lt;Zone&gt;_LO1.ini</code> file, up to 14 slots each (8 base, plus 6
        more as Mnemonic Retention is leveled). Your live gem-slot assignment is separate, server-tracked character state this can't read
        or write.
      </p>
      <p class="mb-2 text-[11px] text-muted-foreground">
        Loads the real, saved spell loadouts from your own <code class="rounded bg-muted px-1 py-0.5">&lt;Character&gt;_&lt;Zone&gt;_LO1.ini</code>
        file (their `UI_`-prefixed counterparts are window layout only, never contents, so they're left out of this list). Edit slots
        below and hit save to write the change back -- a backup of the file as it was before your most recent save is always kept
        alongside it, named the same plus <code class="rounded bg-muted px-1 py-0.5">.eqlp-backup</code>.
      </p>
      {#if uiFilesError}
        <p class="text-[12px] text-destructive">{uiFilesError}</p>
      {:else if !hotbuttonFiles}
        <p class="text-[12px] text-muted-foreground">Loading…</p>
      {:else if !hotbuttonFiles.length}
        <p class="text-[12px] text-muted-foreground">No spellbook files found in your game folder yet.</p>
      {:else}
        <Select.Root type="single" value={selectedFile} onValueChange={(v) => v && loadFile(v)}>
          <Select.Trigger class="h-7 w-72 text-[12px]">{selectedFile ? fileLabel(hotbuttonFiles.find((f) => f.file === selectedFile)!) : 'choose a file…'}</Select.Trigger>
          <Select.Content>
            {#each hotbuttonFiles as f (f.file)}
              <Select.Item value={f.file}>{fileLabel(f)}</Select.Item>
            {/each}
          </Select.Content>
        </Select.Root>

        {#if spellbookLoadError}
          <p class="mt-2 text-[12px] text-destructive">{spellbookLoadError}</p>
        {:else if spellbookFile}
          <!-- why: every real loadout loaded and shown at once, not one
               picked at a time -- direct correction: this file can hold
               many (a real character had 21), and save always writes
               the whole set back regardless of which one you're
               looking at, so the UI should match that instead of
               hiding the rest behind a picker. -->
          <div class="mt-2 flex flex-wrap items-center gap-2">
            <Button size="sm" variant="outline" class="h-7 text-[11px]" onclick={saveLoadouts} disabled={saving}>
              {saving ? 'overwriting…' : `overwrite ${inUseLoadouts.length} to file`}
            </Button>
            <Button size="sm" variant="outline" class="h-7 text-[11px]" onclick={openSaveAsBox}>
              save {inUseLoadouts.length} new file
            </Button>
            <Button size="sm" variant="outline" class="h-7 text-[11px]" onclick={addNewLoadout}>+ new loadout</Button>
            <span class="text-[11px] text-muted-foreground">{inUseLoadouts.length}/60 loadout slots in use</span>
            {#if savedAt}
              <span class="text-[11px] text-muted-foreground">saved {savedAt.toLocaleTimeString()}</span>
            {/if}
          </div>

          {#if showSaveAsBox}
            <!-- why: real fork, not an overwrite -- copies the source
                 file's other data (HotButtons/Combat/etc.) and its UI_
                 layout counterpart to the new name too, so the new pair
                 is a complete, real, loadable file, not just a loadouts
                 fragment. -->
            <div class="mt-2 flex flex-wrap items-center gap-2 rounded-sm border border-border p-2">
              <span class="text-[11px] text-muted-foreground">new file name (Character_Zone):</span>
              <Input
                bind:value={newStemInput}
                placeholder="Character_Zone"
                class="h-7 w-48 text-[12px]"
                onkeydown={(e) => e.key === 'Enter' && saveLoadoutsAsNewFile()}
              />
              <Button size="sm" variant="outline" class="h-7 text-[11px]" onclick={saveLoadoutsAsNewFile} disabled={savingAs}>
                {savingAs ? 'saving…' : 'save'}
              </Button>
              <Button size="sm" variant="ghost" class="h-7 text-[11px]" onclick={() => (showSaveAsBox = false)}>cancel</Button>
              {#if saveAsError}
                <p class="w-full text-[12px] text-destructive">{saveAsError}</p>
              {/if}
            </div>
          {/if}

          {#if loadoutActionError}
            <p class="mt-2 text-[12px] text-destructive">{loadoutActionError}</p>
          {/if}

          <!-- why: capped to ~2-2.5 loadouts tall with its own scroll,
               not the whole page -- a real file can hold 20+, and the
               spell picker below needs to stay in reach for editing any
               of them, not just whichever happened to land near the
               bottom of a long unbounded list. -->
          <div class="mt-2 flex max-h-[420px] flex-col gap-2 overflow-y-auto pr-1">
            {#each inUseLoadouts as lo (lo.index)}
              <!-- why: shrink-0 is load-bearing -- a flex-col container's
                   children shrink to fit by default, which was squeezing
                   every loadout's own slot grid down to fit inside
                   max-h-[420px] instead of letting the container scroll
                   past them (the real bug: only each block's name/delete
                   row, which resists collapsing, stayed visible). -->
              <div class="shrink-0 rounded-sm border border-border p-2">
                <div class="mb-1.5 flex flex-wrap items-center gap-2">
                  <Input
                    value={lo.name ?? ''}
                    oninput={(e) => renameLoadout(lo.index, e.currentTarget.value)}
                    class="h-7 max-w-48 text-[12px]"
                  />
                  <span class="text-[11px] text-muted-foreground">#{lo.index}</span>
                  <Button size="sm" variant="outline" class="h-6 text-[11px]" onclick={() => clearLoadout(lo.index)}>clear</Button>
                  <Button size="sm" variant="outline" class="h-6 text-[11px]" onclick={() => suggestLoadoutSoloBuff(lo.index)}>
                    suggest solo buff
                  </Button>
                  <Button size="sm" variant="outline" class="h-6 text-[11px]" onclick={() => suggestLoadoutTeamBuff(lo.index)}>
                    suggest team buff
                  </Button>
                  <Button size="sm" variant="outline" class="h-6 text-[11px]" onclick={() => suggestLoadoutCombat(lo.index)}>
                    suggest combat
                  </Button>
                  <Button
                    size="sm"
                    variant="ghost"
                    class="h-6 text-[11px] text-destructive"
                    onclick={() => deleteLoadout(lo.index)}
                  >
                    delete
                  </Button>
                </div>
                <div class="grid grid-cols-4 gap-1.5 sm:grid-cols-7">
                  {#each lo.slots as s (s.slot)}
                    {@const isArmed = armed?.loadoutIndex === lo.index && armed.slot === s.slot}
                    <div class="flex flex-col gap-0.5">
                      <span class="text-[9px] text-muted-foreground">{s.slot}</span>
                      {#if s.name}
                        <button
                          type="button"
                          class="flex h-10 flex-col items-center justify-center rounded-sm border border-primary/40 bg-primary/10 px-1 text-center text-[10px] text-foreground hover:border-destructive hover:bg-destructive/10 hover:text-destructive"
                          title={s.catalog_id ? 'click to clear' : 'click to clear (not found in Game Data)'}
                          ondragover={(e) => e.preventDefault()}
                          ondrop={(e) => onLoadoutSlotDrop(e, lo.index, s.slot)}
                          onclick={() => clearLoadoutSlot(lo.index, s.slot)}
                        >
                          {s.name}
                        </button>
                      {:else}
                        <button
                          type="button"
                          class="flex h-10 items-center justify-center rounded-sm border border-dashed text-[10px] {isArmed
                            ? 'border-primary text-primary'
                            : 'border-border text-muted-foreground hover:border-primary hover:text-primary'}"
                          ondragover={(e) => e.preventDefault()}
                          ondrop={(e) => onLoadoutSlotDrop(e, lo.index, s.slot)}
                          onclick={() => (armed = { loadoutIndex: lo.index, slot: s.slot })}
                        >
                          {isArmed ? 'drop here' : 'empty'}
                        </button>
                      {/if}
                    </div>
                  {/each}
                </div>
              </div>
            {/each}
          </div>
        {/if}
      {/if}
      {/if}
    </CardContent>
  </Card>

  <!-- why: right below "Found spellbooks" (capped/scrollable above) --
       asked directly for this: the picker needs to stay in easy reach
       while editing any real loadout, not just whichever one happens to
       land near the bottom of the page. Drag any result onto any slot
       above, or click one to fill whichever slot was last clicked via
       `armed`. The class toggles here also drive the suggest buttons on
       each loadout above, not just this search list. -->
  <Card class="rounded-sm">
    <CardContent class="px-3 py-2.5">
      <div class="mb-1.5 flex items-center justify-between gap-2">
        <button type="button" class="flex items-center gap-1.5 text-left" onclick={() => (suggestedOpen = !suggestedOpen)}>
          <span class="w-6 text-[26px] leading-none font-bold text-foreground">{suggestedOpen ? '▾' : '▸'}</span>
          <h2 class="panel-title">Suggested spells</h2>
        </button>
        {#if suggestedOpen}
          <div class="flex items-center gap-2">
            <button
              type="button"
              class="text-[11px] text-muted-foreground underline-offset-2 hover:text-primary hover:underline"
              onclick={() => openSpellLinePriority(null)}
            >
              customize spell line priority →
            </button>
            <Input bind:value={search} placeholder="search spells…" class="h-7 w-56 text-[12px]" />
          </div>
        {/if}
      </div>

      {#if suggestedOpen}
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
        Drag a result onto any slot above{#if armed}, or click one to fill "{loadoutByIndex(armed.loadoutIndex)?.name}", slot
          {armed.slot}{/if}.
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
      {/if}
    </CardContent>
  </Card>

  <DpsSuggest />
</div>
