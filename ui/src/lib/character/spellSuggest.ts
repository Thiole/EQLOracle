// why: shared spell-suggestion logic for the Spellbook builder's Suggest
// buttons and the DPS calculator's rotation panel -- kept in one place so
// neither ever disagrees with the other about what's usable, a duplicate
// spell line, or mutually exclusive.
import type { SpellDto, SpellClassDto, DamageSpellDto, SpellEffectsEntryDto } from '../tauri/api';
import { MAX_CHARACTER_LEVEL } from './constants';

// ---------------------------------------------------------------- usability

/** why: real bug precedent (dpscalc/SpellbookBuilder) -- data has genuine level 51-60 entries this game's 50 cap can't learn yet */
export function usableClasses(classes: SpellClassDto[]): SpellClassDto[] {
  return classes.filter((c) => c.level == null || c.level <= MAX_CHARACTER_LEVEL);
}

export function isUsable(s: { classes: SpellClassDto[] }): boolean {
  return usableClasses(s.classes).length > 0;
}

/** why: real bug -- the Suggest buttons used bare `isUsable` (any class
 * at all) as their pool filter, `activeClasses` only affected sort
 * order, never which spells were even eligible -- a real "Suggest
 * Combat" run mixed a Druid DoT, a Beastlord DoT, and a spell tagged to
 * "A Freed Soul" (not even a real playable class) into one Enchanter's
 * book. Strict when `activeClasses` is set (the normal case); falls
 * back to level-only when it's genuinely empty/unknown, the same
 * "no filter selected" convention already used elsewhere (e.g. the
 * picker's own class-toggle row). */
export function usableByClasses(classes: SpellClassDto[], activeClasses: string[]): boolean {
  const usable = usableClasses(classes);
  if (!activeClasses.length) return usable.length > 0;
  return usable.some((c) => activeClasses.includes(c.class));
}

// ---------------------------------------------------------------- classification

/** why: spell_type's beneficial-flavored values (~20 distinct real values total) -- everything else is combat by exclusion */
export const BENEFICIAL_TYPES = new Set([
  'Beneficial', 'Statistic Buff', 'Resist Buff', 'Utility Beneficial', 'Heal', 'Heal Over Time',
  'Pet Buff', 'Pet Heal', 'Haste', 'Cure', 'Movement Buff', 'Remove Curse',
]);

/** why: real bug -- 167 real Beneficial-typed spells (food/water/reagent/
 * tradeskill-bar conjuring, "Enchant Gold" etc.) are Summon effects, not
 * stat buffs; anchored at the start of the effect text on purpose --
 * "Limit Effect: Exclude Summon Item" (a real AA clause on genuine buffs
 * like Reagent Conservation) must not false-positive on the substring. */
export function isSummon(s: SpellDto): boolean {
  return s.slots.some((sl) => /^Summon/i.test(sl.effect));
}

/** why: real bug -- Translocate/Teleport/Gate/Circle-of/Portal lines are
 * all Beneficial-typed with a Single/Group target, so they'd otherwise
 * pass as a solo/team buff; these are movement, not a combat/support buff. */
export function isTeleport(s: SpellDto): boolean {
  return s.slots.some((sl) => /^(Translocate|Teleport|Evacuate|Gate)\b/i.test(sl.effect));
}

export function isBuff(s: SpellDto): boolean {
  return !!s.spell_type && BENEFICIAL_TYPES.has(s.spell_type) && !isSummon(s) && !isTeleport(s);
}

/** why: lands on you or one other friendly, not a whole group */
export const SOLO_TARGET_TYPES = new Set(['Self', 'Single', 'Single Friendly (or Self)']);
export function isSoloTarget(s: SpellDto): boolean {
  return !!s.target_type && SOLO_TARGET_TYPES.has(s.target_type);
}

/** why: real target_type values that spread a buff to the whole group, confirmed against packs/spells.json */
export const TEAM_TARGET_TYPES = new Set(['Group', 'Group v1', 'Group v2', 'Party']);
export function isTeamTarget(s: SpellDto): boolean {
  return !!s.target_type && TEAM_TARGET_TYPES.has(s.target_type);
}

// ---------------------------------------------------------------- effect-aware support ranking

/** why: a damage spell's own `resist` field names what it checks --
 * "Cold (-10)" -> "Cold". "Unresistable" (no type at all) and a missing
 * field both mean there's nothing to line a debuff's own resist-decrease
 * up against. */
export function resistTypeOf(raw: string | null): string | null {
  if (!raw || raw.toLowerCase() === 'unresistable') return null;
  const m = raw.match(/^(\w+)/);
  return m ? m[1] : null;
}

/** why: a debuff's own slots name exactly what it decreases -- Malosi
 * has four such slots (Fire/Cold/Magic/Poison), Tashania has one
 * (Magic only), confirmed against packs/spells.json directly. Used to
 * weigh a resist debuff against what the character's own rotation
 * actually needs, not just its level. */
export function decreasedResistTypes(s: SpellDto): string[] {
  const types: string[] = [];
  for (const sl of s.slots) {
    const m = sl.effect.match(/^Decrease (\w+) Resist\b/i);
    if (m) types.push(m[1]);
  }
  return types;
}

export type SupportCategory = 'control' | 'offense_debuff' | 'resist_debuff' | 'dispel' | 'other';

/** why: real bug -- ranking every debuff by level alone treated a
 * Mesmerize/Stun (real survivability, target-independent) the same as a
 * resist debuff (only useful if the rotation's own damage checks that
 * resist) and the same as a reactive dispel (situational at best).
 * Checked in this order since a spell can plausibly match more than one
 * -- the more decisive category wins. */
export function supportCategoryOf(s: SpellDto, effectsEntry: SpellEffectsEntryDto | undefined): SupportCategory {
  // why: Root has no dedicated spelleffect.rs control label yet (real
  // example: Paralyzing Earth's own slot is the literal word "Root")
  if ((effectsEntry?.control.length ?? 0) > 0 || s.slots.some((sl) => /^Root\b/i.test(sl.effect))) return 'control';
  if (s.slots.some((sl) => /^Decrease (Attack Speed|ATK)\b/i.test(sl.effect))) return 'offense_debuff';
  if (decreasedResistTypes(s).length > 0) return 'resist_debuff';
  if (s.slots.some((sl) => /^Cancel Magic\b/i.test(sl.effect))) return 'dispel';
  return 'other';
}

/** why: a debuff you can actually leave slotted beats one that expires
 * in seconds, at the same level -- a minor tiebreaker, not a filter
 * (spelleffect.rs's own duration parse, already shipped via the
 * spellEffects store, no new data needed). */
export function isPersistentDuration(effectsEntry: SpellEffectsEntryDto | undefined): boolean {
  if (!effectsEntry) return false;
  return effectsEntry.duration.is_permanent || (effectsEntry.duration.max_secs ?? 0) >= 60;
}

// ---------------------------------------------------------------- exclusivity

/** why: strips a trailing rank numeral so different tiers of the same
 * spell line group under one key -- a pure grouping key, never shown or
 * treated as a rank claim (two tiers of the same line would occupy the
 * same spellbook slot anyway, so grouping them is safe either way). */
export function lineKey(name: string): string {
  const parts = name.split(' ');
  const tail = parts[parts.length - 1];
  return tail.length > 0 && /^[IVXLCDM]+$/.test(tail) ? parts.slice(0, -1).join(' ') : name;
}

/** why: real bug -- many EQ spell lines rename entirely across level
 * tiers (Allure/Beguile/Cajoling Whispers/Charm are all the same Charm
 * line; Reconstitute/Renewal/Reparation/Restoration/Resurrection/
 * Resuscitate/Revive are all Revive) -- `lineKey`'s roman-numeral
 * stripping never catches this. Their wiki `description` is the same
 * boilerplate with only the level/amount numbers swapped ("Charm up to
 * level 51" vs "...level 46"), so stripping digits from the description
 * groups them correctly -- verified against packs/spells.json directly
 * (184 real multi-member groups, spot-checked for cross-line false
 * positives: none of the ones checked mixed unrelated effects, only
 * legitimately shared multi-class lines like Cancel Magic/Resist Cold). */
export function spellLineKey(s: SpellDto): string {
  return s.description ? s.description.replace(/\d+/g, '#').trim() : lineKey(s.name);
}

/** why: real self-illusion spells -- Illusions category or an "Illusion:"
 * slot effect. Detrimental illusion-flavored curses (Bone Melt etc.)
 * never reach this check since callers already filter to isBuff first. */
export function isIllusion(s: SpellDto): boolean {
  return s.categories.includes('Illusions') || s.slots.some((sl) => /^Illusion/i.test(sl.effect));
}

/** why: real correction -- automatic grouping (spellLineKey) is deliberately
 * conservative (same wiki description only), so it can't know that two
 * differently-flavored, cross-class spells overwrite each other in the
 * real game (Slow from one class vs. another's equivalent) -- there's no
 * scraped data that proves that generically, and guessing by "similar
 * effect" risks false merges (Tash and Malosi are both resist-decrease
 * debuffs but are NOT the same line and must never be merged automatically).
 * So this is 100% a manual assertion: a spell name mapped here is pulled
 * out of its natural line and grouped under the target line's key instead,
 * wherever the player has said on the priority settings page "these
 * overwrite each other." Never guessed, never inferred. */
export function effectiveLineKey(s: SpellDto, customMembership: Record<string, string>): string {
  return customMembership[s.name] ?? spellLineKey(s);
}

/** why: tracks what a suggestion pass can't repeat -- same spell line
 * (natural or a manually-asserted cross-class merge, see
 * effectiveLineKey), a second Illusion, or a real stacking-group
 * conflict (see stackingdata.rs's own doc for why that last one only
 * covers 48 legacy-carryover spells, not a general rule). */
export interface ExclusivityContext {
  usedLineKeys: Set<string>;
  hasIllusion: boolean;
  usedStackingGroups: Set<number>;
}

export function conflictsWithExisting(
  s: SpellDto,
  ctx: ExclusivityContext,
  groups: Record<string, number>,
  customMembership: Record<string, string> = {},
): boolean {
  if (ctx.usedLineKeys.has(effectiveLineKey(s, customMembership))) return true;
  if (ctx.hasIllusion && isIllusion(s)) return true;
  const g = groups[s.name];
  return g != null && ctx.usedStackingGroups.has(g);
}

function commit(
  s: SpellDto,
  ctx: ExclusivityContext,
  groups: Record<string, number>,
  customMembership: Record<string, string> = {},
): void {
  ctx.usedLineKeys.add(effectiveLineKey(s, customMembership));
  if (isIllusion(s)) ctx.hasIllusion = true;
  const g = groups[s.name];
  if (g != null) ctx.usedStackingGroups.add(g);
}

export function buildExclusivityContext(
  existingNames: string[],
  byName: Map<string, SpellDto>,
  groups: Record<string, number>,
  customMembership: Record<string, string> = {},
): ExclusivityContext {
  const ctx: ExclusivityContext = { usedLineKeys: new Set(), hasIllusion: false, usedStackingGroups: new Set() };
  for (const name of existingNames) {
    const s = byName.get(name);
    if (s) {
      commit(s, ctx, groups, customMembership);
    } else {
      // why: catalog lookup miss shouldn't lose the dedup entirely -- still block an exact-shape repeat by name
      ctx.usedLineKeys.add(lineKey(name));
    }
  }
  return ctx;
}

// ---------------------------------------------------------------- spell lines

/** why: every real multi-member spell line in the catalog, for the
 * priority settings page -- 2+ members only (a 1-member "line" has
 * nothing to reorder). Members pre-sorted by the default (-level) order
 * so a line with no saved override still displays sensibly. */
export interface SpellLine {
  key: string;
  label: string;
  members: SpellDto[];
}

function byDefaultLevelDesc(a: SpellDto, b: SpellDto): number {
  const la = Math.max(0, ...a.classes.map((c) => c.level ?? 0));
  const lb = Math.max(0, ...b.classes.map((c) => c.level ?? 0));
  return lb - la;
}

/** why: real correction -- the line's own wiki description read like log
 * text ("Causes your opponent to fall into an enchanted sleep…"), not a
 * real name. `members` is already sorted highest-level first, so its own
 * name is the natural label -- stable regardless of any priority
 * override (a re-ranked line's label doesn't change just because its
 * *suggested* pick did; those are two different questions). */
function lineLabel(members: SpellDto[]): string {
  return members[0]?.name ?? '';
}

/** why: every spell sharing `key`'s effective line, any size (including
 * a not-yet-merged singleton) -- used to open a spell's line detail
 * panel directly from a name search, not just from allSpellLines' own
 * 2+-member browse list (a brand new custom merge always starts from
 * at least one side that has nothing to browse to yet). */
export function membersOfLine(
  spells: SpellDto[],
  key: string,
  customMembership: Record<string, string> = {},
): SpellDto[] {
  return spells.filter((s) => effectiveLineKey(s, customMembership) === key).sort(byDefaultLevelDesc);
}

export function allSpellLines(spells: SpellDto[], customMembership: Record<string, string> = {}): SpellLine[] {
  const groups = new Map<string, SpellDto[]>();
  for (const s of spells) {
    const key = effectiveLineKey(s, customMembership);
    (groups.get(key) ?? groups.set(key, []).get(key)!).push(s);
  }
  const lines: SpellLine[] = [];
  for (const [key, members] of groups) {
    if (members.length < 2) continue;
    const sorted = [...members].sort(byDefaultLevelDesc);
    lines.push({ key, label: lineLabel(sorted), members: sorted });
  }
  return lines.sort((a, b) => a.label.localeCompare(b.label));
}

/** why: an override's array index if the player has manually ranked
 * this spell's (effective, post-merge) line and included this spell;
 * else the same -level fallback sortForSuggestion always used, so an
 * untouched line's order is unchanged. Offset so overridden entries
 * (always small indices) naturally sort ahead of any unconfigured
 * member of a partially-ranked line. */
export function priorityRank(
  s: SpellDto,
  overrides: Record<string, string[]>,
  customMembership: Record<string, string> = {},
): number {
  const order = overrides[effectiveLineKey(s, customMembership)];
  if (order) {
    const idx = order.indexOf(s.name);
    if (idx >= 0) return idx;
  }
  const level = Math.max(0, ...s.classes.map((c) => c.level ?? 0));
  return 1000 - level;
}

// ---------------------------------------------------------------- buff/support picking

type SortKey = [number, number, number, string, string];

/** why: light default ranking, mirrors SpellbookBuilder's own picker
 * heuristic -- known-class membership first (real bug precedent: raw
 * level sorting buries a played class's own mid-level spells under
 * other classes' level-60 raid content), then a manually-ranked spell
 * line's saved priority if one exists (real bug: Mesmerization, an AE
 * mez, is the better pick despite later single-target mez spells
 * outranking it by level -- see priorityRank's own doc), falling back
 * to highest usable level, then class/spell name for a stable order. */
function sortForSuggestion(
  s: SpellDto,
  activeClasses: string[],
  preferSoloTarget: boolean,
  overrides: Record<string, string[]>,
  customMembership: Record<string, string>,
): SortKey {
  const usable = usableClasses(s.classes);
  const known = usable.filter((c) => activeClasses.includes(c.class));
  const pool = known.length ? known : usable;
  const bestClass = pool.length ? [...pool].sort((a, b) => a.class.localeCompare(b.class))[0].class : '';
  const soloTier = preferSoloTarget ? (isSoloTarget(s) ? 0 : 1) : 1;
  return [soloTier, known.length ? 0 : 1, priorityRank(s, overrides, customMembership), bestClass, s.name];
}

function compareSortKeys(a: SortKey, b: SortKey): number {
  for (let i = 0; i < a.length; i++) {
    if (a[i] < b[i]) return -1;
    if (a[i] > b[i]) return 1;
  }
  return 0;
}

/** why: greedily fills up to `count` empty slots from `pool`, skipping
 * anything that would conflict (same line, a second Illusion, a real
 * stacking-group clash) with the book's existing contents or an
 * already-picked suggestion this same call. */
export function pickBuffSuggestions(
  pool: SpellDto[],
  activeClasses: string[],
  existingBookNames: string[],
  count: number,
  groups: Record<string, number>,
  overrides: Record<string, string[]> = {},
  customMembership: Record<string, string> = {},
): string[] {
  if (count <= 0) return [];
  const candidates = pool.filter((s) => usableByClasses(s.classes, activeClasses));
  const byName = new Map(candidates.map((s) => [s.name, s]));
  const ctx = buildExclusivityContext(existingBookNames, byName, groups, customMembership);
  const sorted = [...candidates].sort((a, b) =>
    compareSortKeys(
      sortForSuggestion(a, activeClasses, true, overrides, customMembership),
      sortForSuggestion(b, activeClasses, true, overrides, customMembership),
    ),
  );
  const picked: string[] = [];
  for (const s of sorted) {
    if (picked.length >= count) break;
    if (conflictsWithExisting(s, ctx, groups, customMembership)) continue;
    picked.push(s.name);
    commit(s, ctx, groups, customMembership);
  }
  return picked;
}

const SUPPORT_CATEGORY_ORDER: SupportCategory[] = ['control', 'offense_debuff', 'resist_debuff', 'dispel', 'other'];

/** why: real correction -- ranking every debuff by level alone put a
 * resist debuff (only useful if the rotation's own damage checks that
 * resist type -- Tash strips Magic only, Malosi strips Fire/Cold/Magic/
 * Poison) on equal footing with real target-independent survivability
 * (Mez/Stun/Root) and a purely reactive dispel. Classifies by
 * `supportCategoryOf`, drops an irrelevant resist debuff entirely
 * (`rotationResistTypes` empty means "no info", not "nothing's
 * relevant" -- stays permissive rather than wiping every resist debuff
 * out for a caller that didn't pass rotation data), then round-robins
 * one pick per category per pass so survivability and a rotation-
 * relevant resist debuff both get a real shot instead of one category
 * (usually whichever has the single highest-level member) monopolizing
 * every slot. `damageSpellNames` excludes anything that's actually a
 * DPS spell (Rend/Conflagration etc. are `spell_type: "Detrimental"`
 * too, despite being pure nukes, not support). */
export function pickSupportSuggestions(
  pool: SpellDto[],
  effects: Record<string, SpellEffectsEntryDto>,
  activeClasses: string[],
  existingBookNames: string[],
  count: number,
  groups: Record<string, number>,
  damageSpellNames: Set<string>,
  overrides: Record<string, string[]> = {},
  customMembership: Record<string, string> = {},
  rotationResistTypes: Set<string> = new Set(),
): string[] {
  if (count <= 0) return [];
  const byName = new Map(pool.map((s) => [s.name, s]));
  const ctx = buildExclusivityContext(existingBookNames, byName, groups, customMembership);
  const usable = pool.filter(
    (s) =>
      usableByClasses(s.classes, activeClasses) &&
      s.spell_type === 'Detrimental' &&
      !damageSpellNames.has(s.name),
  );

  const candidates = usable.filter((s) => {
    if (supportCategoryOf(s, effects[s.id]) !== 'resist_debuff') return true;
    if (rotationResistTypes.size === 0) return true;
    return decreasedResistTypes(s).some((t) => rotationResistTypes.has(t));
  });

  function compareWithinCategory(a: SpellDto, b: SpellDto): number {
    if (supportCategoryOf(a, effects[a.id]) === 'resist_debuff') {
      const overlapA = decreasedResistTypes(a).filter((t) => rotationResistTypes.has(t)).length;
      const overlapB = decreasedResistTypes(b).filter((t) => rotationResistTypes.has(t)).length;
      if (overlapA !== overlapB) return overlapB - overlapA;
    }
    const pa = isPersistentDuration(effects[a.id]) ? 0 : 1;
    const pb = isPersistentDuration(effects[b.id]) ? 0 : 1;
    if (pa !== pb) return pa - pb;
    return compareSortKeys(
      sortForSuggestion(a, activeClasses, false, overrides, customMembership),
      sortForSuggestion(b, activeClasses, false, overrides, customMembership),
    );
  }

  const byCategory = new Map<SupportCategory, SpellDto[]>(SUPPORT_CATEGORY_ORDER.map((c) => [c, []]));
  for (const s of candidates) byCategory.get(supportCategoryOf(s, effects[s.id]))!.push(s);
  for (const list of byCategory.values()) list.sort(compareWithinCategory);

  const picked: string[] = [];
  let progress = true;
  while (picked.length < count && progress) {
    progress = false;
    for (const cat of SUPPORT_CATEGORY_ORDER) {
      if (picked.length >= count) break;
      const list = byCategory.get(cat)!;
      while (list.length) {
        const s = list.shift()!;
        if (conflictsWithExisting(s, ctx, groups, customMembership)) continue;
        picked.push(s.name);
        commit(s, ctx, groups, customMembership);
        progress = true;
        break;
      }
    }
  }
  return picked;
}

// ---------------------------------------------------------------- rotation simulator

export interface RotationResult {
  sequence: DamageSpellDto[];
  totalDamage: number;
  avgDps: number;
}

/** why: mirrors dpscalc.rs's own TICK_SECS -- a DoT's tick cadence, needed
 * here to spread its lifetime damage across real tick landing times
 * instead of crediting it all at the cast moment (see simulateRotation). */
const TICK_SECS = 6;

/** why: real correction -- a DoT (and a multi-wave AE like Frost Storm,
 * loosely) keeps dealing damage *after* the cast completes, on its own
 * clock, independent of whatever gets cast next; crediting its whole
 * lifetime total at the instant of casting over-counts when a DoT is
 * cast near the window's end (ticks past the window haven't actually
 * landed yet) and was the wrong number for "damage after 15/60s".
 * Schedules each tick at its real landing time and only counts ones
 * that land at or before `windowSecs`. A nuke (including a multi-wave
 * one) has no separate tick data to schedule -- its whole total still
 * lands at cast-complete, same as before. */
function scheduleDamage(spell: DamageSpellDto, castStart: number, castEnd: number, windowSecs: number): number {
  if (!spell.is_dot || spell.duration_secs == null) {
    return castEnd <= windowSecs ? spell.total_damage : 0;
  }
  let credited = castEnd <= windowSecs ? spell.instant_damage : 0;
  const tickDamage = spell.total_damage - spell.instant_damage;
  const ticks = Math.max(1, Math.round(spell.duration_secs / TICK_SECS));
  const perTick = tickDamage / ticks;
  for (let i = 1; i <= ticks; i++) {
    const tickTime = castEnd + i * TICK_SECS;
    if (tickTime <= windowSecs) credited += perTick;
  }
  return credited;
}

/** why: real bug, caught from a live regression report -- a static
 * total_damage/casting_time (the old `castValue`) massively overstates
 * a long-duration DoT's real worth once the window can't fit all its
 * ticks. Real example: rank-10 Plague (1220 lifetime dmg over a 117s
 * duration, 1.8s cast) scored 677.8 by the old metric -- better than
 * Conflagration's 360 -- but a 60s window can only ever land ~9 of its
 * 20 ticks, so its *real* realizable value is closer to 340, no better
 * than Rend. The old metric greedily grabbed it anyway, on real data
 * displacing two full Conflagration casts (3600 real damage) for one
 * Plague cast that only delivered a fraction of its nominal total --
 * a real, reported DPS regression when a class with long DoTs got added.
 * `realizedValueAt` asks the right question instead: "if cast starting
 * *right now*, how much of this spell's damage would actually land
 * before the window ends" -- reuses `scheduleDamage`, so a nuke (never
 * truncated) scores the same as before, and Frost Storm's own weave-in
 * behavior (see the old castValue doc, now folded into this one) is
 * unaffected since its single-cast damage always lands whole. */
function realizedValueAt(s: DamageSpellDto, castStart: number, windowSecs: number): number {
  return scheduleDamage(s, castStart, castStart + s.casting_time, windowSecs) / s.casting_time;
}

/** why: greedy timeline scheduler -- at each point the caster is free,
 * cast whichever *ready* spell pays the most per second of casting time
 * committed right now, given how much of it would actually land before
 * the window ends (`realizedValueAt` -- see its own doc for why a raw
 * total/casting_time comparison over-values a DoT the window can't fit).
 * Generalizes the old single best-nuke-vs-worthwhile-DoT and 2-nuke
 * weave-pair heuristics into a real N-spell, real-timeline simulation --
 * not a global optimum (a true schedule optimizer is a much harder
 * problem), but validated against real data: alternating Rend/
 * Conflagration beats spamming either alone, weaving Frost Storm in
 * wherever it's ready beats skipping it, and a long DoT no longer
 * displaces real nuke damage for a payoff it can't actually deliver,
 * all reproduced by this same greedy rule on their own. A DoT already
 * cast keeps ticking on its own clock while other spells get woven in
 * (see scheduleDamage) -- it just can't be recast until its own
 * duration fully resolves (`nextAvailable`), so it never overwrites/
 * stacks with itself. No pool cap -- the loop is cheap regardless of
 * candidate count (bounded by windowSecs / shortest cast time, not by
 * pool size squared), and real class combinations easily exceed what a
 * small cap could safely pre-filter (58+ usable damage spells for just
 * two classes, confirmed against the real catalog). */
export function simulateRotation(candidates: DamageSpellDto[], windowSecs: number): RotationResult {
  const pool = candidates.filter((s) => s.casting_time > 0 && s.casting_time <= windowSecs);

  const nextAvailable = new Map<string, number>();
  const sequence: DamageSpellDto[] = [];
  let totalDamage = 0;
  let t = 0;
  // why: weaving between two DIFFERENT spells isn't zero-gap
  // back-to-back once one is off its own reuse -- roughly half of the
  // just-cast spell's recast_time is a real minimum before any next
  // cast (an estimate, not measured off log timestamps). A single
  // scalar floor, not per-spell: the caster is mid-recovery, not the
  // specific spell; each spell's FULL reuse is still tracked separately
  // via nextAvailable, this only adds a floor on top.
  //
  // Capped at GCD_FLOOR_CAP_SECS -- real bug, reproduced against the
  // real candidate pool (A/B harness on the reference log's own DTOs):
  // uncapped, a 12s-recast AE (Frost Storm) locked the caster out of
  // EVERYTHING for 6s after each cast, and a 15s-window rotation
  // degraded from Frost Storm -> Conflagration -> Ice Comet (426 dps)
  // to Frost Storm -> Mana Detonation (260 dps) -- the "wrong spells"
  // regression reported live. The half-of-recast estimate was
  // calibrated on standard 1.5s-recast nukes (0.75s gap); the measured
  // weave transitions in BACKLOG.md (GMMS->Ice Comet 2.16s cycles,
  // EM->GMMS 1.77s over 489 samples) say the real inter-cast gap is
  // small and roughly FLAT, not proportional to the previous spell's
  // own cooldown -- so the cap keeps the estimate exactly as-is for
  // ordinary nukes and stops it scaling with long AE recasts.
  const GCD_FLOOR_CAP_SECS = 0.75;
  let gcdFloor = 0;

  while (t < windowSecs) {
    const cursor = Math.max(t, gcdFloor);
    const ready = pool.filter(
      (s) => (nextAvailable.get(s.name) ?? 0) <= cursor && cursor + s.casting_time <= windowSecs,
    );
    if (ready.length === 0) {
      const future = pool.filter((s) => cursor + s.casting_time <= windowSecs);
      if (future.length === 0) break;
      const nextT = Math.max(cursor, Math.min(...future.map((s) => nextAvailable.get(s.name) ?? 0)));
      if (nextT <= cursor) break; // defensive -- should be unreachable, avoids ever looping in place
      t = nextT;
      continue;
    }
    const best = ready.reduce((a, b) =>
      (realizedValueAt(b, cursor, windowSecs) > realizedValueAt(a, cursor, windowSecs) ? b : a),
    );
    const castStart = cursor;
    sequence.push(best);
    t = castStart + best.casting_time;
    gcdFloor = t + Math.min(best.recast_time / 2, GCD_FLOOR_CAP_SECS);
    totalDamage += scheduleDamage(best, castStart, t, windowSecs);
    // why: total_damage / dps_with_reuse == cycle_secs (casting + recast,
    // or duration for a DoT) -- already-shipped fields, no new backend data
    const cycleSecs = best.total_damage / best.dps_with_reuse;
    nextAvailable.set(best.name, castStart + cycleSecs);
  }

  return { sequence, totalDamage, avgDps: windowSecs > 0 ? totalDamage / windowSecs : 0 };
}

export interface SequenceChip {
  spell: DamageSpellDto;
  count: number;
}

/** why: display helper -- a fast-cycling spell repeated many times reads
 * better as "Conflagration ×4" than four separate chips in a row. */
export function collapseSequence(sequence: DamageSpellDto[]): SequenceChip[] {
  const chips: SequenceChip[] = [];
  for (const s of sequence) {
    const last = chips[chips.length - 1];
    if (last && last.spell.name === s.name) {
      last.count += 1;
    } else {
      chips.push({ spell: s, count: 1 });
    }
  }
  return chips;
}
