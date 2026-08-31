<script lang="ts">
  import * as Select from '$lib/components/ui/select';
  import { Card, CardContent } from '$lib/components/ui/card';
  import { Button } from '$lib/components/ui/button';
  import { Input } from '$lib/components/ui/input';
  import { Separator } from '$lib/components/ui/separator';
  import {
    race,
    activeClasses,
    levels,
    userLevels,
    defaultClasses,
    estimate,
    setRace,
    toggleActiveClass,
    setLevel,
    estimateLevelsFromLog,
  } from '$lib/stores/character';
  import { ALL_CLASSES, ALL_RACES, MAX_ACTIVE_CLASSES } from './constants';

  // why: display order only -- ALL_RACES itself mirrors gearplanner.rs's own order
  const sortedRaces = [...ALL_RACES].sort();

  function row(label: string, val: number | null | undefined, suffix = ''): [string, string] {
    return [label, val == null ? '—' : Math.round(val) + suffix];
  }

  const vitalsRows = $derived(
    $estimate
      ? [
          row('HP', $estimate.vitals.hp),
          row('Mana', $estimate.total_mana),
          row('Endurance', $estimate.vitals.endurance),
          null,
          row('AC', $estimate.vitals.ac),
          row('Attack', $estimate.vitals.attack),
          row('Velocity', $estimate.vitals.velocity),
          null,
          row('HP Regen', $estimate.vitals.hp_regen),
          row('Mana Regen', $estimate.vitals.mana_regen),
          row('End Regen', $estimate.vitals.end_regen),
        ]
      : null,
  );

  const statResistRows = $derived(
    $estimate
      ? (() => {
          const attrTotal = (code: string) => $estimate!.attrs.find((r) => r.attr === code)?.total ?? null;
          return [
            row('Str', attrTotal('STR')),
            row('Stam', attrTotal('STA')),
            row('Int', attrTotal('INT')),
            row('Wis', attrTotal('WIS')),
            row('Agi', attrTotal('AGI')),
            row('Dex', attrTotal('DEX')),
            row('Cha', attrTotal('CHA')),
            null,
            row('SV Magic', $estimate!.resists.magic),
            row('SV Fire', $estimate!.resists.fire),
            row('SV Cold', $estimate!.resists.cold),
            row('SV Disease', $estimate!.resists.disease),
            row('SV Poison', $estimate!.resists.poison),
            row('SV Void', $estimate!.resists.void),
          ];
        })()
      : null,
  );
</script>

<div class="flex flex-col gap-3">
  <Card>
    <CardContent class="px-3 py-2">
      <h2 class="text-[11px] uppercase tracking-wide text-muted-foreground">Character</h2>
      <p class="mb-2 text-[11px] text-muted-foreground">
        What's confirmed from your own parsed log, plus race — the log never states that directly, so it's set here by hand.
      </p>
      <label class="flex max-w-xs items-center gap-2 text-[12px]">
        <span class="shrink-0 {$race ? 'text-muted-foreground' : 'font-medium text-primary'}">race</span>
        <Select.Root type="single" value={$race} onValueChange={(v) => setRace(v ?? '')}>
          <Select.Trigger class="h-7 flex-1 text-[12px] {$race ? '' : 'border-primary ring-1 ring-primary/40'}">{$race || '— not set —'}</Select.Trigger>
          <Select.Content>
            {#each sortedRaces as r (r)}
              <Select.Item value={r}>{r}</Select.Item>
            {/each}
          </Select.Content>
        </Select.Root>
      </label>
      <p class="mt-2 text-[11px] text-muted-foreground">
        {#if $defaultClasses.length}
          Confirmed class configuration: {$defaultClasses.join(' / ')}
        {:else}
          No confirmed class configuration yet — fight a bit and check back.
        {/if}
      </p>
    </CardContent>
  </Card>

  <Card>
    <CardContent class="px-3 py-2">
      <div class="mb-2 flex items-center justify-between">
        <h2 class="text-[11px] uppercase tracking-wide text-muted-foreground">Character Planner</h2>
        <Button
          size="sm"
          variant="secondary"
          class="h-6 text-[11px]"
          onclick={estimateLevelsFromLog}
          title="Fill every class's level with your most recently observed character level, as a starting guess to correct by hand"
        >
          Estimate levels
        </Button>
      </div>

      <div class="grid grid-flow-col grid-rows-4 gap-x-4 gap-y-0.5">
        {#each ALL_CLASSES as c (c)}
          {@const on = $activeClasses.includes(c)}
          {@const atCap = $activeClasses.length >= MAX_ACTIVE_CLASSES}
          {@const userSet = c in $userLevels}
          <div class="flex items-center gap-1.5 py-0.5">
            <button
              type="button"
              class="flex-1 truncate rounded-md border px-1.5 py-0.5 text-left text-[11px] transition-colors {on
                ? 'border-primary/50 bg-primary/15 text-primary'
                : atCap
                  ? 'cursor-not-allowed border-border text-muted-foreground opacity-40'
                  : 'border-border hover:bg-accent'}"
              disabled={!on && atCap}
              onclick={() => toggleActiveClass(c)}
              title={on || !atCap ? 'Mark as one of your 3 currently active classes' : 'Already at 3 active — every class plays exactly 3 at once'}
            >
              {c}
            </button>
            <!-- why: brass ring + dot = "set by you" -- a typed level is
                 persisted and never re-estimated; the estimate keeps
                 filling only untouched classes. See stores/character.ts
                 userLevels' own doc. -->
            <div class="relative">
              <Input
                type="number"
                min="1"
                max="50"
                class="h-6 w-14 px-1.5 text-[11px] tabular-nums {userSet ? 'border-primary/60' : ''}"
                value={$levels[c] ?? 1}
                title={userSet
                  ? 'Set by you — kept across launches. "Estimate levels" hands it back to the estimator.'
                  : 'Estimated from the log — type a level to set it yourself.'}
                oninput={(e) => setLevel(c, Number((e.target as HTMLInputElement).value))}
              />
              {#if userSet}
                <span class="absolute -top-0.5 -right-0.5 size-1.5 rounded-full bg-primary" aria-hidden="true"></span>
              {/if}
            </div>
          </div>
        {/each}
      </div>
      {#if Object.keys($userLevels).length}
        <p class="mt-1.5 text-[10px] text-muted-foreground">
          <span class="mr-0.5 inline-block size-1.5 rounded-full bg-primary align-middle"></span>
          set by you — kept across launches; “Estimate levels” resets all of them
        </p>
      {/if}

      <Separator class="my-2" />

      {#if !$race}
        <p class="text-[12px] text-muted-foreground">Set a race above — race base attributes are needed before class adds and mana can be shown.</p>
      {:else if !$activeClasses.length}
        <p class="text-[12px] text-muted-foreground">Mark up to 3 classes above as active to see a full sheet.</p>
      {:else if !$estimate}
        <p class="text-[12px] text-muted-foreground">Loading…</p>
      {:else}
        {@const est = $estimate}
        {#if est.classes.length < 3}
          <p class="mb-1 text-[11px] text-muted-foreground">
            {3 - est.classes.length} active class slot{est.classes.length === 2 ? '' : 's'} empty — totals below only count what's marked active.
          </p>
        {/if}
        <p class="mb-1 text-[12px]">
          Character level <b class="tabular-nums">{est.character_level}</b>
          {#if est.limiting_class}
            — capped by <b>{est.limiting_class}</b>
          {:else if est.classes.length > 1}
            — {est.classes.length} classes tied at the lowest
          {/if}
        </p>

        <div class="overflow-x-auto">
          <table class="w-full text-[11px]">
            <thead>
              <tr class="border-b border-border text-muted-foreground">
                <th class="px-2 py-0.5 text-left font-normal"></th>
                <th class="px-2 py-0.5 text-right font-normal">Base</th>
                {#each est.classes as c (c)}
                  <th class="px-2 py-0.5 text-right font-normal">{c}</th>
                {/each}
                <th class="px-2 py-0.5 text-right font-normal">Naked</th>
                <th class="px-2 py-0.5 text-right font-normal">Gear</th>
                <th class="px-2 py-0.5 text-right font-normal">Total</th>
              </tr>
            </thead>
            <tbody>
              {#each est.attrs as r (r.attr)}
                {@const over = r.total > est.attr_cap}
                <tr class="border-b border-border/50">
                  <td class="px-2 py-0.5">{r.attr}</td>
                  <td class="px-2 py-0.5 text-right tabular-nums text-muted-foreground">{r.base}</td>
                  {#each r.class_adds as v, i (est.classes[i])}
                    <td class="px-2 py-0.5 text-right tabular-nums text-muted-foreground">{v ? '+' + v : '·'}</td>
                  {/each}
                  <td class="px-2 py-0.5 text-right tabular-nums">{r.naked}</td>
                  <td class="px-2 py-0.5 text-right tabular-nums text-primary">{r.gear ? (r.gear > 0 ? '+' : '') + Math.round(r.gear) : '·'}</td>
                  <td class="px-2 py-0.5 text-right tabular-nums font-medium {over ? 'text-caution' : ''}">{Math.round(r.total)}{over ? '‡' : ''}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>

        <div class="mt-2 space-y-1 text-[11px] text-muted-foreground">
          {#if est.mana.length}
            <p>
              <b class="text-foreground">Mana:</b>
              {est.mana.map((m) => `${m.class} (${m.casting_stat}) ${Math.round(m.pool)}${m.counted ? '' : ' (not counted)'}`).join(', ')}
              — pool comes from your <b class="text-foreground">two highest</b> classes only, total
              <b class="text-foreground tabular-nums">{Math.round(est.total_mana)}</b>. Includes gear.
            </p>
          {/if}
          {#if est.attrs.some((r) => r.total > est.attr_cap)}
            <p class="text-caution">‡ over the reported {est.attr_cap} ceiling. Players report it; it isn't confirmed in the client, so nothing here is clamped.</p>
          {/if}
          {#if est.bad_class_adds.length}
            <p class="text-caution"><b>chardata is off:</b> {est.bad_class_adds.join(', ')} don't add up to what every other class does.</p>
          {/if}
          <p>Attribute numbers are <b class="text-foreground">classic EQ values, unverified for EQL</b> — eqlwiki doesn't publish them. Treat this as an estimate, not a promise.</p>
        </div>
      {/if}
    </CardContent>
  </Card>

  <div class="grid grid-cols-2 gap-3">
    <Card>
      <CardContent class="px-3 py-2">
        <h2 class="mb-1 text-[11px] uppercase tracking-wide text-muted-foreground">Char Vitals</h2>
        {#if vitalsRows}
          <div class="grid grid-cols-[1fr_auto] gap-x-2 gap-y-0.5 text-[12px]">
            {#each vitalsRows as r, i (i)}
              {#if r === null}
                <div class="col-span-2 my-1 border-t border-border"></div>
              {:else}
                <div class="text-muted-foreground">{r[0]}</div>
                <div class="text-right tabular-nums">{r[1]}</div>
              {/if}
            {/each}
          </div>
        {:else}
          <p class="text-[12px] text-muted-foreground">No estimate yet.</p>
        {/if}
      </CardContent>
    </Card>
    <Card>
      <CardContent class="px-3 py-2">
        <h2 class="mb-1 text-[11px] uppercase tracking-wide text-muted-foreground">Stat / Resist</h2>
        {#if statResistRows}
          <div class="grid grid-cols-[1fr_auto] gap-x-2 gap-y-0.5 text-[12px]">
            {#each statResistRows as r, i (i)}
              {#if r === null}
                <div class="col-span-2 my-1 border-t border-border"></div>
              {:else}
                <div class="text-muted-foreground">{r[0]}</div>
                <div class="text-right tabular-nums">{r[1]}</div>
              {/if}
            {/each}
          </div>
        {:else}
          <p class="text-[12px] text-muted-foreground">No estimate yet.</p>
        {/if}
      </CardContent>
    </Card>
  </div>
</div>
