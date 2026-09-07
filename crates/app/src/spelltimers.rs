//! why: the game's own spell data for what the wiki gets wrong or lacks
//! -- the install's `spells_us.txt` (173 `^` columns). Read here: cast
//! time (col 8, ms), recast (col 10, ms) and the shared reuse timer id
//! (col 55). Verified 2026-09-03 on the real file: Lifebite cast 1750
//! recast 1500 where the wiki page has neither; Spike/Spear of
//! Disease/Spear of Pain share timer 22, the rains 3, Conflagration 0.
//! input: the install folder; output: per spell name

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

const CAST_COL: usize = 8;
const RECAST_COL: usize = 10;
const TIMER_ID_COL: usize = 55;
/// why: 16 per-class level requirements, 255 = that class cannot cast it
/// (L8/G6). Verified on the real file: Conflagration WIZ 43, Lifedraw
/// NEC 12 / SHD 15, Improved Invisibility WIZ 55 and ENC 50.
const CLASS_LEVEL_COL: usize = 36;
/// why: buff duration -- formula (col 12) and base (col 13). Both 0 is
/// an instant: Harvest, Ice Comet, Gate. Every buff has one or the other
/// (Clarity 3/270, Vampiric Embrace 50/0, Selo's 5/2).
const DURATION_FORMULA_COL: usize = 12;
const DURATION_COL: usize = 13;
/// why: the effect list, `$`-separated slots of `slot|SPA|base1|base2|max|formula`.
/// The stacking rule lives here: same slot + same SPA conflicts, SPA 148/149
/// are blockers. Validated against every "did not take hold (Blocked by X)"
/// in a real log -- 1,814 of 1,814 explained (2026-09-07).
const EFFECTS_COL: usize = 173;
/// why: the file's own class order, classic-EQ, not this app's alphabetical
const FILE_CLASSES: [&str; 16] = [
    "Warrior",
    "Cleric",
    "Paladin",
    "Ranger",
    "Shadow Knight",
    "Druid",
    "Monk",
    "Bard",
    "Rogue",
    "Shaman",
    "Necromancer",
    "Wizard",
    "Magician",
    "Enchanter",
    "Beastlord",
    "Berserker",
];
/// why: the file's "no" value for a class column
const NOT_CASTABLE: u8 = 255;

/// why: one effect slot off the file. SPA is the effect type (15 mana
/// regen, 11 haste, 1 AC, 69 max HP, 85 add proc, 121 heal on hit ...).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SlotEffect {
    pub slot: u8,
    pub spa: u16,
    pub base1: f64,
    pub base2: f64,
    pub max: f64,
    pub formula: u32,
}

impl SlotEffect {
    /// why: `SPA 10 base 0` is padding that lines a real effect up in a
    /// chosen slot number -- 15,740 spells use it. A real CHA buff is SPA
    /// 10 with a nonzero base, and does conflict.
    pub fn is_spacer(&self) -> bool {
        self.spa == 10 && self.base1 == 0.0 && self.base2 == 0.0 && self.formula == 0
    }

    /// why: the sign is the difference between a buff and its debuff --
    /// Weaken is STR -10, a DoT is HP -10 per tick, Tashina is MR -5.
    /// Attack speed is a percentage: Celerity 128, a slow 80 or 90, so
    /// there the line is 100. Regeneration is base 0 growing by formula,
    /// which is why 0 stays in.
    pub fn is_beneficial(&self) -> bool {
        if self.spa == 11 {
            return self.base1 > 100.0;
        }
        self.base1 >= 0.0
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SpellFileEntry {
    pub cast_ms: u32,
    pub recast_ms: u32,
    /// why: 0 means no shared timer
    pub timer: u32,
    /// why: level required per class, `FILE_CLASSES` order, 255 = never
    pub levels: [u8; 16],
    /// why: false is an instant -- a heal, a nuke, Harvest -- never a buff
    pub has_duration: bool,
    /// why: columns 12/13, see `duration_ticks`
    pub duration_formula: u32,
    pub duration_base: u32,
    pub slots: Vec<SlotEffect>,
}

impl SpellFileEntry {
    /// why: the file's own duration, in ticks (6 s), at a given caster
    /// level -- columns 12/13 are a formula id and a base, the classic
    /// table (EQEmu's CalcBuffDuration_formula). Verified on the real
    /// file: Clarity 3/270 -> 270 at 50, Spirit of Wolf 3/360 -> 360,
    /// Berserker Spirit 7/50 -> 50, Shadow Compact 1/4 -> 4, Selo's
    /// 5/2 -> 2, Vampiric Embrace 50/0 -> permanent, Harvest 0/0 -> 0.
    pub fn duration_ticks(&self, level: u32) -> u32 {
        let (f, b) = (self.duration_formula, self.duration_base);
        let capped = |i: u32| if b > 0 { i.min(b) } else { i };
        match f {
            0 => 0,
            1 => capped(level.div_ceil(2)),
            2 => capped(if level > 3 { level.div_ceil(2) + 5 } else { 6 }),
            3 => capped(level * 30),
            4 => capped(50),
            5 => capped(2),
            6 => capped(level / 2 + 2),
            7 => capped(level),
            8 => capped(level + 10),
            9 => capped(level * 2 + 10),
            10 => capped(level * 3 + 10),
            50 => 72_000,
            3600 => {
                if b > 0 {
                    b
                } else {
                    3600
                }
            }
            _ => b,
        }
    }

    /// why: the game's own stacking rule, the one the "did not take hold"
    /// line enforces: a shared slot number carrying the same SPA, or a
    /// 148/149 blocker on either side aimed at the other's SPA and slot.
    /// Spacers never conflict. Different slots, or different SPAs in one
    /// slot, stack -- Vampiric Embrace (85) and Blessing of the Squire
    /// (121) share slot 1 and stack.
    pub fn conflicts(&self, other: &SpellFileEntry) -> bool {
        let real = |e: &SpellFileEntry| -> Vec<SlotEffect> {
            e.slots.iter().copied().filter(|s| !s.is_spacer()).collect()
        };
        let (a, b) = (real(self), real(other));
        let shared = a.iter().any(|x| {
            b.iter()
                .any(|y| x.slot == y.slot && x.spa == y.spa && x.spa != 148 && x.spa != 149)
        });
        let blocks = |from: &[SlotEffect], to: &[SlotEffect]| {
            from.iter()
                .filter(|s| s.spa == 148 || s.spa == 149)
                .any(|s| {
                    to.iter()
                        .any(|t| f64::from(t.spa) == s.base1 && f64::from(t.slot) == s.base2)
                })
        };
        shared || blocks(&a, &b) || blocks(&b, &a)
    }
}

pub type SpellFile = Arc<HashMap<String, SpellFileEntry>>;

fn cache() -> &'static Mutex<HashMap<PathBuf, SpellFile>> {
    static C: OnceLock<Mutex<HashMap<PathBuf, SpellFile>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

fn parse(text: &str) -> HashMap<String, SpellFileEntry> {
    let mut out = HashMap::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.split('^').collect();
        if fields.len() <= TIMER_ID_COL {
            continue;
        }
        let name = fields[1];
        let num = |i: usize| fields[i].parse::<u32>().unwrap_or(0);
        let mut levels = [NOT_CASTABLE; 16];
        for (i, slot) in levels.iter_mut().enumerate() {
            if let Some(f) = fields.get(CLASS_LEVEL_COL + i) {
                *slot = f.parse::<u8>().unwrap_or(NOT_CASTABLE);
            }
        }
        let slots = fields
            .get(EFFECTS_COL - 1)
            .map(|e| parse_slots(e))
            .unwrap_or_default();
        let entry = SpellFileEntry {
            cast_ms: num(CAST_COL),
            recast_ms: num(RECAST_COL),
            timer: num(TIMER_ID_COL),
            levels,
            has_duration: num(DURATION_FORMULA_COL - 1) != 0 || num(DURATION_COL - 1) != 0,
            duration_formula: num(DURATION_FORMULA_COL - 1),
            duration_base: num(DURATION_COL - 1),
            slots,
        };
        // why: the file repeats some names, the later row often a
        // non-castable stub (every class 255) -- keep the row that
        // actually states levels rather than whichever came first
        match out.entry(name.to_ascii_lowercase()) {
            std::collections::hash_map::Entry::Vacant(v) => {
                v.insert(entry);
            }
            std::collections::hash_map::Entry::Occupied(mut o) => {
                let have = o.get().levels.iter().any(|l| *l != NOT_CASTABLE);
                if !have && levels.iter().any(|l| *l != NOT_CASTABLE) {
                    o.insert(entry);
                }
            }
        }
    }
    out
}

/// why: `1|15|1|0|109|9$2|...` -- a slot the file leaves blank (SPA -1
/// or empty) is skipped, a malformed one too; base values can be floats
fn parse_slots(text: &str) -> Vec<SlotEffect> {
    text.split('$')
        .filter_map(|part| {
            let f: Vec<&str> = part.split('|').collect();
            if f.len() < 6 || f[1].is_empty() || f[1] == "-1" {
                return None;
            }
            Some(SlotEffect {
                slot: f[0].parse().ok()?,
                spa: f[1].parse().ok()?,
                base1: f[2].parse().unwrap_or(0.0),
                base2: f[3].parse().unwrap_or(0.0),
                max: f[4].parse().unwrap_or(0.0),
                formula: f[5].parse().unwrap_or(0),
            })
        })
        .collect()
}

/// why: a file built from text -- tests and probes, no install needed
pub fn parse_text(text: &str) -> SpellFile {
    Arc::new(parse(text))
}

/// why: read once per install folder; a missing file reads as an empty
/// map, never an error -- every caller falls back to the wiki pack
pub fn spell_file(base_dir: &Path) -> SpellFile {
    if let Some(t) = cache().lock().ok().and_then(|c| c.get(base_dir).cloned()) {
        return t;
    }
    let text = std::fs::read(base_dir.join("spells_us.txt"))
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default();
    let t: SpellFile = Arc::new(parse(&text));
    if let Ok(mut c) = cache().lock() {
        c.insert(base_dir.to_path_buf(), t.clone());
    }
    t
}

pub fn entry_of(file: &SpellFile, name: &str) -> Option<SpellFileEntry> {
    file.get(&name.to_ascii_lowercase()).cloned()
}

/// why: L8 -- the game's own per-class level requirements for one spell,
/// this server's numbers rather than a wiki scrape. Empty when the spell
/// is unknown or no class can cast it.
pub fn class_levels(file: &SpellFile, name: &str) -> Vec<(String, u8)> {
    let Some(e) = entry_of(file, name) else {
        return Vec::new();
    };
    e.levels
        .iter()
        .enumerate()
        .filter(|(_, l)| **l != NOT_CASTABLE && **l > 0)
        .map(|(i, l)| (FILE_CLASSES[i].to_string(), *l))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: u32, name: &str, cast: u32, recast: u32, timer: u32) -> String {
        let mut f = vec!["0".to_string(); 60];
        f[0] = id.to_string();
        f[1] = name.to_string();
        f[CAST_COL] = cast.to_string();
        f[RECAST_COL] = recast.to_string();
        f[TIMER_ID_COL] = timer.to_string();
        f.join("^")
    }

    /// why: L8's data source -- the install's own per-class levels, and
    /// the duplicate-name stub row must not win over the real one
    #[test]
    fn class_levels_read_from_the_files_own_columns() {
        let mut f = vec!["0".to_string(); 60];
        f[1] = "Improved Invisibility".to_string();
        for i in 0..16 {
            f[CLASS_LEVEL_COL + i] = "255".to_string();
        }
        f[CLASS_LEVEL_COL + 11] = "55".to_string(); // Wizard
        f[CLASS_LEVEL_COL + 13] = "50".to_string(); // Enchanter
        let real = f.join("^");
        let mut stub = vec!["0".to_string(); 60];
        stub[1] = "Improved Invisibility".to_string();
        for i in 0..16 {
            stub[CLASS_LEVEL_COL + i] = "255".to_string();
        }
        let file: SpellFile = Arc::new(parse(&format!("{}\n{}", stub.join("^"), real)));
        let mut got = class_levels(&file, "improved invisibility");
        got.sort();
        assert_eq!(
            got,
            vec![
                ("Enchanter".to_string(), 50u8),
                ("Wizard".to_string(), 55u8)
            ],
            "the stub row must not shadow the row that states levels"
        );
    }

    /// why: the real slot strings off the real file, and the outcomes the
    /// log showed for them -- Boon blocked by Clarity 81 times, Berserker
    /// Strength blocked by Harnessing of Spirit 56 times with no shared
    /// slot at all, and never once anything against Vampiric Embrace
    #[test]
    fn the_slot_rule_reproduces_the_logs_own_blocks() {
        let row = |id: u32, name: &str, slots: &str| {
            let mut f = vec![String::new(); 173];
            f[0] = id.to_string();
            f[1] = name.to_string();
            f[11] = "3".to_string();
            f[172] = slots.to_string();
            f.join("^")
        };
        let text = [
            row(174, "Clarity", "1|10|0|0|100|0$2|15|1|0|109|9"),
            row(1694, "Boon of the Clear Mind", "1|10|0|0|100|0$2|15|1|0|119|9"),
            row(359, "Vampiric Embrace", "1|85|821|0|100|0$2|10|0|0|100|0"),
            row(74009, "Blessing of the Squire", "1|121|2|0|100|0"),
            row(1, "Berserker Strength", "1|4|40|0|100|40$2|55|200|0|100|250$3|6|-20|0|100|20"),
            row(2, "Harnessing of Spirit", "1|69|151|0|103|251$2|79|151|0|103|251$3|10|0|0|100|0$4|4|42|0|101|67$5|5|1|0|102|50$6|149|4|1|100|67$7|149|5|1|100|50$8|148|4|1|100|1067$9|148|5|1|100|1050"),
            row(3, "Radiant Visage", "1|10|10|0|101|30"),
            row(4, "Glamour", "1|10|10|0|101|32"),
        ]
        .join("\n");
        let file = parse_text(&text);
        let e = |n: &str| entry_of(&file, n).expect(n);
        assert!(
            e("Boon of the Clear Mind").conflicts(&e("Clarity")),
            "same slot, same SPA"
        );
        assert!(
            !e("Vampiric Embrace").conflicts(&e("Blessing of the Squire")),
            "same slot, different SPA: stacks"
        );
        assert!(
            e("Berserker Strength").conflicts(&e("Harnessing of Spirit")),
            "no shared slot -- the 148 blocker"
        );
        assert!(
            e("Radiant Visage").conflicts(&e("Glamour")),
            "a real CHA buff is not a spacer"
        );
        assert!(
            !e("Clarity").conflicts(&e("Vampiric Embrace")),
            "spacers never conflict"
        );
        assert!(e("Clarity").has_duration && e("Clarity").slots.len() == 2);
    }

    /// why: the file's own numbers for the durations that decide what
    /// the tracker admits -- see groupbuffs::MIN_BUFF_TICKS
    #[test]
    fn duration_ticks_follows_the_classic_formula_table() {
        let e = |f: u32, b: u32| SpellFileEntry {
            duration_formula: f,
            duration_base: b,
            ..Default::default()
        };
        assert_eq!(e(3, 270).duration_ticks(50), 270, "Clarity");
        assert_eq!(e(3, 360).duration_ticks(50), 360, "Spirit of Wolf");
        assert_eq!(e(3, 360).duration_ticks(5), 150, "SoW at 5: level*30");
        assert_eq!(e(7, 50).duration_ticks(50), 50, "Berserker Spirit");
        assert_eq!(e(1, 4).duration_ticks(50), 4, "Shadow Compact");
        assert_eq!(e(5, 2).duration_ticks(50), 2, "Selo's");
        assert_eq!(e(9, 110).duration_ticks(50), 110, "Alacrity");
        assert_eq!(
            e(50, 0).duration_ticks(50),
            72_000,
            "Vampiric Embrace: permanent"
        );
        assert_eq!(e(0, 0).duration_ticks(50), 0, "Harvest: instant");
    }

    /// why: the real columns, the real groups Spencer named
    #[test]
    fn cast_recast_and_timer_read_from_their_columns() {
        let text = [
            row(1, "Spike of Disease", 500, 45000, 22),
            row(2, "Spear of Pain", 500, 45000, 22),
            row(3, "Lifebite", 1750, 1500, 0),
            row(4, "Conflagration", 5000, 1500, 0),
        ]
        .join("\n");
        let f: SpellFile = Arc::new(parse(&text));
        assert_eq!(entry_of(&f, "Spear of Pain").map(|e| e.timer), Some(22));
        assert_eq!(
            entry_of(&f, "lifebite").map(|e| (e.cast_ms, e.recast_ms)),
            Some((1750, 1500))
        );
        assert_eq!(entry_of(&f, "Conflagration").map(|e| e.timer), Some(0));
        assert_eq!(entry_of(&f, "Nope"), None);
    }
}
