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

/** why: support/control tag family from spelleffect.rs's own categorization -- used to prefer real debuffs/CC over an arbitrary Detrimental spell for the "supporting skills" fill */
export const SUPPORT_TAGS = new Set(['Debuff', 'Slow', 'Snare', 'Mez', 'Charm', 'Fear', 'Stun']);

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

function lineLabel(key: string, members: SpellDto[]): string {
  // why: the line's own description (digits already stripped by
  // spellLineKey) reads better as a label than a joined name list --
  // "Causes your opponent to fall into an enchanted sleep…" identifies
  // the line at a glance, member names show in the detail panel instead.
  return members[0]?.description ? key : members.map((s) => s.name).join(' / ');
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
    lines.push({ key, label: lineLabel(key, sorted), members: sorted });
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

/** why: "for now" heuristic fill for debuffs/CC -- there's no real
 * ranking system for these yet (unlike combat's real DPS math), so this
 * just prefers a tagged support/control effect over a bare Detrimental
 * one, then falls back to the same known-class/level ordering as buffs.
 * `damageSpellNames` excludes anything that's actually a DPS spell
 * (Rend/Conflagration etc. are `spell_type: "Detrimental"` too, despite
 * being pure nukes, not support). */
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
): string[] {
  if (count <= 0) return [];
  const byName = new Map(pool.map((s) => [s.name, s]));
  const ctx = buildExclusivityContext(existingBookNames, byName, groups, customMembership);
  const candidates = pool.filter(
    (s) =>
      usableByClasses(s.classes, activeClasses) &&
      s.spell_type === 'Detrimental' &&
      !damageSpellNames.has(s.name),
  );
  const sorted = [...candidates].sort((a, b) => {
    const ta = effects[a.id]?.tags.some((t) => SUPPORT_TAGS.has(t)) ? 0 : 1;
    const tb = effects[b.id]?.tags.some((t) => SUPPORT_TAGS.has(t)) ? 0 : 1;
    if (ta !== tb) return ta - tb;
    return compareSortKeys(
      sortForSuggestion(a, activeClasses, false, overrides, customMembership),
      sortForSuggestion(b, activeClasses, false, overrides, customMembership),
    );
  });
  const picked: string[] = [];
  for (const s of sorted) {
    if (picked.length >= count) break;
    if (conflictsWithExisting(s, ctx, groups, customMembership)) continue;
    picked.push(s.name);
    commit(s, ctx, groups, customMembership);
  }
  return picked;
}

// ---------------------------------------------------------------- rotation simulator

export interface RotationResult {
  sequence: DamageSpellDto[];
  totalDamage: number;
  avgDps: number;
}

/** why: keeps the per-step scan small -- the best pair/trio is always
 * drawn from the strongest individual spells anyway, same reasoning the
 * old bestWeavePair search already used. */
const ROTATION_POOL_CAP = 10;

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

/** why: real bug -- greedily picking by `dps_with_reuse` (a spell's own
 * *solo*, spammed-forever rate) undervalues a big one-shot nuke whose
 * long downtime other spells can fill anyway. Real numbers exposed it:
 * rank-10 Frost Storm (3072 dmg/5s cast, 12s recast) has a mediocre solo
 * dps_with_reuse (180) next to Rend/Conflagration's (263/342), so the
 * old criterion never picked it even though it was ready -- but Frost
 * Storm's single-cast payoff (3072) dwarfs a single Rend/Conflag cast
 * (1710/1800) for the *same* 5s of commitment. Weaving one FS cast in
 * wherever it's ready, instead of skipping it, raised a 60s window's
 * total from 21,060 to 25,146 on these exact numbers -- confirmed by
 * hand and by re-running this function. */
function castValue(s: DamageSpellDto): number {
  return s.total_damage / s.casting_time;
}

/** why: greedy timeline scheduler -- at each point the caster is free,
 * cast whichever *ready* spell pays the most per second of casting time
 * committed right now (`castValue`), not whichever has the best rate if
 * spammed alone (`dps_with_reuse` -- see castValue's own doc for why
 * that criterion under-weaves a big one-shot nuke). Generalizes the old
 * single best-nuke-vs-worthwhile-DoT and 2-nuke weave-pair heuristics
 * into a real N-spell, real-timeline simulation -- not a global optimum
 * (a true schedule optimizer is a much harder problem), but validated
 * against real data: alternating Rend/Conflagration beats spamming
 * either alone, and weaving Frost Storm in wherever it's ready beats
 * skipping it, both reproduced by this same greedy rule on their own.
 * A DoT already cast keeps ticking on its own clock while other spells
 * get woven in (see scheduleDamage) -- it just can't be recast until
 * its own duration fully resolves (`nextAvailable`), so it never
 * overwrites/stacks with itself. */
export function simulateRotation(candidates: DamageSpellDto[], windowSecs: number): RotationResult {
  const pool = [...candidates]
    .filter((s) => s.casting_time > 0 && s.casting_time <= windowSecs)
    .sort((a, b) => castValue(b) - castValue(a))
    .slice(0, ROTATION_POOL_CAP);

  const nextAvailable = new Map<string, number>();
  const sequence: DamageSpellDto[] = [];
  let totalDamage = 0;
  let t = 0;

  while (t < windowSecs) {
    const ready = pool.filter(
      (s) => (nextAvailable.get(s.name) ?? 0) <= t && t + s.casting_time <= windowSecs,
    );
    if (ready.length === 0) {
      const future = pool.filter((s) => t + s.casting_time <= windowSecs);
      if (future.length === 0) break;
      const nextT = Math.min(...future.map((s) => nextAvailable.get(s.name) ?? 0));
      if (nextT <= t) break; // defensive -- should be unreachable, avoids ever looping in place
      t = nextT;
      continue;
    }
    const best = ready.reduce((a, b) => (castValue(b) > castValue(a) ? b : a));
    const castStart = t;
    sequence.push(best);
    t = castStart + best.casting_time;
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
