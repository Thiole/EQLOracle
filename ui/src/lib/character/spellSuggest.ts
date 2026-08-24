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

// ---------------------------------------------------------------- classification

/** why: spell_type's beneficial-flavored values (~20 distinct real values total) -- everything else is combat by exclusion */
export const BENEFICIAL_TYPES = new Set([
  'Beneficial', 'Statistic Buff', 'Resist Buff', 'Utility Beneficial', 'Heal', 'Heal Over Time',
  'Pet Buff', 'Pet Heal', 'Haste', 'Cure', 'Movement Buff', 'Remove Curse',
]);
export function isBuff(s: SpellDto): boolean {
  return !!s.spell_type && BENEFICIAL_TYPES.has(s.spell_type);
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

/** why: real self-illusion spells -- Illusions category or an "Illusion:"
 * slot effect. Detrimental illusion-flavored curses (Bone Melt etc.)
 * never reach this check since callers already filter to isBuff first. */
export function isIllusion(s: SpellDto): boolean {
  return s.categories.includes('Illusions') || s.slots.some((sl) => /^Illusion/i.test(sl.effect));
}

/** why: tracks what a suggestion pass can't repeat -- same spell line,
 * a second Illusion, or a real stacking-group conflict (see
 * stackingdata.rs's own doc for why that last one only covers 48
 * legacy-carryover spells, not a general rule). */
export interface ExclusivityContext {
  usedLineKeys: Set<string>;
  hasIllusion: boolean;
  usedStackingGroups: Set<number>;
}

export function conflictsWithExisting(
  s: SpellDto,
  ctx: ExclusivityContext,
  groups: Record<string, number>,
): boolean {
  if (ctx.usedLineKeys.has(lineKey(s.name))) return true;
  if (ctx.hasIllusion && isIllusion(s)) return true;
  const g = groups[s.name];
  return g != null && ctx.usedStackingGroups.has(g);
}

function commit(s: SpellDto, ctx: ExclusivityContext, groups: Record<string, number>): void {
  ctx.usedLineKeys.add(lineKey(s.name));
  if (isIllusion(s)) ctx.hasIllusion = true;
  const g = groups[s.name];
  if (g != null) ctx.usedStackingGroups.add(g);
}

export function buildExclusivityContext(
  existingNames: string[],
  byName: Map<string, SpellDto>,
  groups: Record<string, number>,
): ExclusivityContext {
  const ctx: ExclusivityContext = { usedLineKeys: new Set(), hasIllusion: false, usedStackingGroups: new Set() };
  for (const name of existingNames) {
    const s = byName.get(name);
    if (s) {
      commit(s, ctx, groups);
    } else {
      // why: catalog lookup miss shouldn't lose the dedup entirely -- still block an exact-shape repeat by name
      ctx.usedLineKeys.add(lineKey(name));
    }
  }
  return ctx;
}

// ---------------------------------------------------------------- buff/support picking

type SortKey = [number, number, number, string, string];

/** why: light default ranking, mirrors SpellbookBuilder's own picker
 * heuristic -- known-class membership first (real bug precedent: raw
 * level sorting buries a played class's own mid-level spells under
 * other classes' level-60 raid content), then highest usable level,
 * then class/spell name for a stable order. */
function sortForSuggestion(s: SpellDto, activeClasses: string[], preferSoloTarget: boolean): SortKey {
  const usable = usableClasses(s.classes);
  const known = usable.filter((c) => activeClasses.includes(c.class));
  const pool = known.length ? known : usable;
  const level = pool.length ? Math.max(...pool.map((c) => c.level ?? 0)) : 0;
  const bestClass = pool.length ? [...pool].sort((a, b) => a.class.localeCompare(b.class))[0].class : '';
  const soloTier = preferSoloTarget ? (isSoloTarget(s) ? 0 : 1) : 1;
  return [soloTier, known.length ? 0 : 1, -level, bestClass, s.name];
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
): string[] {
  if (count <= 0) return [];
  const byName = new Map(pool.map((s) => [s.name, s]));
  const ctx = buildExclusivityContext(existingBookNames, byName, groups);
  const sorted = [...pool].sort((a, b) =>
    compareSortKeys(sortForSuggestion(a, activeClasses, true), sortForSuggestion(b, activeClasses, true)),
  );
  const picked: string[] = [];
  for (const s of sorted) {
    if (picked.length >= count) break;
    if (conflictsWithExisting(s, ctx, groups)) continue;
    picked.push(s.name);
    commit(s, ctx, groups);
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
): string[] {
  if (count <= 0) return [];
  const byName = new Map(pool.map((s) => [s.name, s]));
  const ctx = buildExclusivityContext(existingBookNames, byName, groups);
  const candidates = pool.filter(
    (s) => isUsable(s) && s.spell_type === 'Detrimental' && !damageSpellNames.has(s.name),
  );
  const sorted = [...candidates].sort((a, b) => {
    const ta = effects[a.id]?.tags.some((t) => SUPPORT_TAGS.has(t)) ? 0 : 1;
    const tb = effects[b.id]?.tags.some((t) => SUPPORT_TAGS.has(t)) ? 0 : 1;
    if (ta !== tb) return ta - tb;
    return compareSortKeys(sortForSuggestion(a, activeClasses, false), sortForSuggestion(b, activeClasses, false));
  });
  const picked: string[] = [];
  for (const s of sorted) {
    if (picked.length >= count) break;
    if (conflictsWithExisting(s, ctx, groups)) continue;
    picked.push(s.name);
    commit(s, ctx, groups);
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

/** why: greedy timeline scheduler -- at each point the caster is free,
 * cast whichever ready spell has the best `dps_with_reuse` (its own
 * steady-state rate, already correct for both nukes and DoTs since that
 * field's denominator already encodes "never recast/refresh early").
 * Generalizes the old single best-nuke-vs-worthwhile-DoT and 2-nuke
 * weave-pair heuristics into a real N-spell, real-timeline simulation --
 * not a global optimum (a true schedule optimizer is a much harder
 * problem), but validated against real data during design: alternating
 * Rend/Conflagration (5s cast, 1.5s recast each) beats spamming either
 * alone, exactly the pattern this reproduces on its own. */
export function simulateRotation(candidates: DamageSpellDto[], windowSecs: number): RotationResult {
  const pool = [...candidates]
    .filter((s) => s.casting_time > 0 && s.casting_time <= windowSecs)
    .sort((a, b) => b.dps_with_reuse - a.dps_with_reuse)
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
    const best = ready.reduce((a, b) => (b.dps_with_reuse > a.dps_with_reuse ? b : a));
    const castStart = t;
    sequence.push(best);
    totalDamage += best.total_damage;
    t = castStart + best.casting_time;
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
