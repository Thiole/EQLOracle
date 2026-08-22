// why: mirrors gearplanner.rs's CLASS_NAMES/RACE_NAMES, not exposed over IPC

export const ALL_CLASSES = [
  'Bard', 'Beastlord', 'Berserker', 'Cleric',
  'Druid', 'Enchanter', 'Magician', 'Monk',
  'Necromancer', 'Paladin', 'Ranger', 'Rogue',
  'Shadow Knight', 'Shaman', 'Warrior', 'Wizard',
] as const;

export const ALL_RACES = [
  'Human', 'Barbarian', 'Erudite', 'Wood Elf', 'High Elf', 'Dark Elf', 'Halfling', 'Dwarf',
  'Troll', 'Ogre', 'Gnome', 'Iksar', 'Vah Shir', 'Froglok', 'Half Elf',
] as const;

// why: every class plays exactly 3 at once above level 10
export const MAX_ACTIVE_CLASSES = 3;

// why: this game's real character level cap -- shared so anything that
// judges a spell/AA/etc. as "actually learnable right now" uses the same
// number `stores/character.ts`'s own level clamp already enforces,
// instead of a second, driftable magic number.
export const MAX_CHARACTER_LEVEL = 50;

// why: mirrors gearplanner.rs's SLOTS const
export const SLOT_LABELS: Record<string, string> = {
  EAR1: 'Ear', HEAD: 'Head', FACE: 'Face', EAR2: 'Ear', NECK: 'Neck',
  SHOULDERS: 'Shoulders', ARMS: 'Arms', BACK: 'Back', WRIST1: 'Wrist', WRIST2: 'Wrist',
  RANGE: 'Range', HANDS: 'Hands', PRIMARY: 'Primary', SECONDARY: 'Secondary',
  FINGER1: 'Finger', FINGER2: 'Finger', CHEST: 'Chest', LEGS: 'Legs', FEET: 'Feet',
  WAIST: 'Waist', AMMO: 'Ammo', ANY1: 'Any', ANY2: 'Any',
};

/** why: doll grid layout, 4x6; input: none; output: slot key + label, null=spacer */
export const DOLL_ROWS: ReadonlyArray<ReadonlyArray<[string, string] | null>> = [
  [['EAR1', 'Ear'], ['NECK', 'Neck'], ['FACE', 'Face'], ['HEAD', 'Head'], ['EAR2', 'Ear'], null],
  [['FINGER1', 'Finger'], ['WRIST1', 'Wrist'], ['ARMS', 'Arm'], ['HANDS', 'Hands'], ['WRIST2', 'Wrist'], ['FINGER2', 'Finger']],
  [['SHOULDERS', 'Shldr'], ['CHEST', 'Chest'], ['BACK', 'Back'], ['WAIST', 'Waist'], ['LEGS', 'Legs'], ['FEET', 'Feet']],
  [['PRIMARY', 'Prim'], ['SECONDARY', 'Sec'], ['RANGE', 'Range'], ['AMMO', 'Ammo'], ['ANY1', 'Any'], ['ANY2', 'Any']],
];

/** why: icon path, matches legacy planner/icons/ layout */
export const ICON_BASE = '/planner/icons/';

// why: derived_weights's stat keys, grouped for display
export const WEIGHT_GROUPS: Array<[string, Array<[string, string]>]> = [
  ['stats', [['AC', 'AC'], ['HP', 'HP'], ['MANA', 'Mana']]],
  ['attributes', [['STR', 'Str'], ['STA', 'Sta'], ['AGI', 'Agi'], ['DEX', 'Dex'], ['WIS', 'Wis'], ['INT', 'Int'], ['CHA', 'Cha']]],
  ['other', [['RATIO', 'Wep Ratio'], ['EFFECT', 'Focus/Click/Worn']]],
];

// why: full class name -> 3-letter code, mirrors gearplanner.rs's CLASS_NAMES
export const CLASS_CODE: Record<string, string> = {
  Warrior: 'WAR', Cleric: 'CLR', Paladin: 'PAL', Ranger: 'RNG', 'Shadow Knight': 'SHD',
  Druid: 'DRU', Monk: 'MNK', Bard: 'BRD', Rogue: 'ROG', Shaman: 'SHM',
  Necromancer: 'NEC', Wizard: 'WIZ', Magician: 'MAG', Enchanter: 'ENC', Beastlord: 'BST', Berserker: 'BER',
};

/** why: archetype grouping, for tag color only -- not a game mechanic
 *  input: class code; output: 'melee' | 'hybrid' | 'priest' | 'caster' */
export const CLASS_ARCHETYPE: Record<string, 'melee' | 'hybrid' | 'priest' | 'caster'> = {
  WAR: 'melee', MNK: 'melee', ROG: 'melee', BER: 'melee',
  PAL: 'hybrid', SHD: 'hybrid', RNG: 'hybrid', BRD: 'hybrid', BST: 'hybrid',
  CLR: 'priest', DRU: 'priest', SHM: 'priest',
  WIZ: 'caster', MAG: 'caster', NEC: 'caster', ENC: 'caster',
};

export const ARCHETYPE_COLOR: Record<string, string> = {
  melee: '#8b9bb4',
  hybrid: '#cc8a5c',
  priest: '#7fc4a8',
  caster: '#b090e0',
};

/** why: best-effort archetype-AA -> eligible classes, not in the scrape data
 *  input: real EQ class-gating knowledge; output: hint only, never hides rows.
 *  A handful of archetype AAs aren't listed here at all -- not confident
 *  enough in which classes get them to guess, so they show with no tag
 *  rather than a wrong one. */
export const ARCHETYPE_AA_CLASSES: Record<string, string[]> = {
  Ambidexterity: ['WAR', 'MNK', 'ROG', 'BER', 'RNG', 'BST'],
  'Burst of Power': ['WAR', 'MNK', 'ROG', 'BER', 'RNG', 'BST'],
  'Double Riposte': ['WAR', 'PAL', 'SHD', 'MNK', 'ROG', 'BER', 'RNG', 'BST', 'BRD'],
  'Finishing Blow': ['WAR', 'PAL', 'SHD', 'MNK', 'ROG', 'BER', 'RNG', 'BST', 'BRD'],
  Acrobatics: ['MNK', 'ROG', 'BRD', 'BST'],
  'Physical Enhancement': ['WAR', 'PAL', 'SHD', 'RNG', 'MNK', 'ROG', 'BRD', 'BER', 'BST'],
  'Mental Clarity': ['WIZ', 'MAG', 'NEC', 'ENC', 'CLR', 'DRU', 'SHM', 'PAL', 'RNG', 'SHD', 'BRD', 'BST'],
  'Mnemonic Retention': ['WIZ', 'MAG', 'NEC', 'ENC', 'CLR', 'DRU', 'SHM', 'PAL', 'RNG', 'SHD', 'BRD', 'BST'],
  'Persistent Casting': ['WIZ', 'MAG', 'NEC', 'ENC', 'CLR', 'DRU', 'SHM', 'PAL', 'RNG', 'SHD', 'BRD', 'BST'],
  'Mastery of the Past': ['WIZ', 'MAG', 'NEC', 'ENC', 'CLR', 'DRU', 'SHM', 'PAL', 'RNG', 'SHD', 'BRD', 'BST'],
  'Spell Casting Mastery': ['WIZ', 'MAG', 'NEC', 'ENC', 'CLR', 'DRU', 'SHM', 'PAL', 'RNG', 'SHD', 'BRD', 'BST'],
  'Spell Casting Deftness': ['WIZ', 'MAG', 'NEC', 'ENC', 'CLR', 'DRU', 'SHM', 'PAL', 'RNG', 'SHD', 'BRD', 'BST'],
  'Spell Casting Reinforcement': ['WIZ', 'MAG', 'NEC', 'ENC', 'CLR', 'DRU', 'SHM', 'PAL', 'RNG', 'SHD', 'BRD', 'BST'],
  'Spell Casting Subtlety': ['WIZ', 'MAG', 'NEC', 'ENC', 'CLR', 'DRU', 'SHM', 'PAL', 'RNG', 'SHD', 'BRD', 'BST'],
  'Master of All': ['WIZ', 'MAG', 'NEC', 'ENC', 'CLR', 'DRU', 'SHM'],
  'Destructive Fury': ['WIZ', 'MAG', 'NEC', 'DRU', 'SHM', 'ENC'],
  'Fury of Magic': ['WIZ', 'MAG', 'NEC', 'DRU', 'SHM', 'ENC'],
  'Quick Damage': ['WIZ', 'MAG', 'NEC', 'DRU', 'SHM', 'ENC'],
  'Critical Affliction': ['NEC', 'DRU', 'SHM'],
  'Destructive Cascade': ['NEC', 'DRU', 'SHM'],
  'Healing Adept': ['CLR', 'DRU', 'SHM', 'PAL', 'RNG', 'BST'],
  'Healing Gift': ['CLR', 'DRU', 'SHM', 'PAL', 'RNG', 'BST'],
  'Healing Boon': ['CLR', 'DRU', 'SHM'],
  "Companion's Discipline": ['NEC', 'MAG', 'BST', 'SHD', 'DRU', 'RNG'],
  'Mend Companion': ['NEC', 'MAG', 'BST', 'SHD', 'DRU', 'RNG'],
  'Pet Affinity': ['NEC', 'MAG', 'BST', 'SHD', 'DRU', 'RNG'],
};
