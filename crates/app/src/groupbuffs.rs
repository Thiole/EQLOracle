//! why: Group Buff Tracker -- "Good" when every beneficial buff the
//! party's confirmed classes can cast on you, that benefits your own
//! class combo, is currently on you; else the missing ones and who could
//! cast them. Party = the group tracker's current roster (pets excluded);
//! a class is confirmed by a /who row in this chain or by the class
//! detector's own bar; a level comes from /who only (unknown = any rank).

use crate::ingest::Ingest;
use crate::spelldata::{spells, Spell};
use eqlp_session::classdetect::LEVEL_CAP;
use eqlp_session::Kind;
use eqlp_source::Millis;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};

/// why: the buff LINES that matter -- one entry per effect kind, not per
/// rank; any active spell of the kind covers it
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub enum BuffKind {
    ManaRegen,
    Haste,
    Hp,
    Ac,
    HpRegen,
    Resist,
    Movement,
    Attack,
    Strength,
    Dexterity,
    Stamina,
    Agility,
    DamageShield,
    /// why: "Increase Absorb Damage" -- a rune. Barrier of Force was
    /// filing under MANA REGEN, because absorb matched no arm and the
    /// walk fell through to the incidental 3 mana/tick on its third slot.
    Rune,
    /// why: a familiar. Lesser Familiar was filing under RESISTS off its
    /// first slot, when what you want tracked is "have I got my familiar
    /// out". EQ names them plainly and there are two, both Wizard self.
    Familiar,
    /// why: "Add Proc" -- Vampiric Embrace and its line. A real beneficial
    /// self-buff that mapped to no kind, so it could never be reported
    /// missing however plainly the log showed it absent.
    Proc,
    /// why: SPA 121, a reverse damage shield -- "heal yourself per
    /// successful melee hit". The cleric Blessing of the Page/Squire/
    /// Knight/Lord Commander line. The wiki's slot prose said "Add
    /// Proc_Strike", which filed it with Vampiric Embrace's lifetap proc
    /// (SPA 85); the game file says they are different effects that
    /// stack, and a Shadow Knight wants both.
    HealOnHit,
}

impl BuffKind {
    pub fn label(self) -> &'static str {
        match self {
            BuffKind::Rune => "rune",
            BuffKind::Familiar => "familiar",
            BuffKind::Proc => "weapon proc",
            BuffKind::HealOnHit => "heal on hit",
            BuffKind::ManaRegen => "mana regen",
            BuffKind::Haste => "haste",
            BuffKind::Hp => "hit points",
            BuffKind::Ac => "armor class",
            BuffKind::HpRegen => "hp regen",
            BuffKind::Resist => "resists",
            BuffKind::Movement => "run speed",
            BuffKind::Attack => "attack",
            BuffKind::Strength => "strength",
            BuffKind::Dexterity => "dexterity",
            BuffKind::Stamina => "stamina",
            BuffKind::Agility => "agility",
            BuffKind::DamageShield => "damage shield",
        }
    }
}

/// why: the first recognizable slot decides the kind; procs, levitate,
/// see-invis and the rest are situational and never "missing"
/// why: the game file first, the wiki's slot prose only when there is no
/// file (tests, the mock harness, an install without one). The file's
/// SPA is what an effect IS; the prose is a scrape of it. Real case:
/// "Add Proc_Strike" text filed the cleric heal-on-hit line under the
/// same kind as a lifetap proc, and two spells the game lets you stack
/// read as one row with an "upgrade" between them. An instant (no
/// duration) is never a buff, which is the "per tick" test the text arms
/// hand-roll. An SPA this map does not name falls through to the prose,
/// so nothing the text could classify is lost.
pub fn kind_of_with(
    spell: &Spell,
    file: Option<&crate::spelltimers::SpellFile>,
) -> Option<BuffKind> {
    if spell.name.ends_with("Familiar") {
        return Some(BuffKind::Familiar);
    }
    let text = kind_of(spell);
    let Some(entry) = file.and_then(|f| crate::spelltimers::entry_of(f, &spell.name)) else {
        return text;
    };
    // why: Spencer -- "short term buffs should not be in here. group
    // buffs should be tracking buffs you cast out of combat". A 4-tick HP
    // transfer (Shadow Compact), a 2-tick song, a mez, a root: things you
    // fire, not things you keep up. Judged at the cap so the answer does
    // not move with the caster's level.
    if entry.duration_ticks(MAINTAINED_AT_LEVEL) < MIN_BUFF_TICKS {
        return None;
    }
    let spas: Vec<u16> = entry
        .slots
        .iter()
        .filter(|s| !s.is_spacer() && s.is_beneficial())
        .map(|s| s.spa)
        .collect();
    // why: the file CORRECTS and FILLS, it does not reorder. A buff's
    // slots run in the file's order and in the wiki's, and those differ
    // (Shielding is HP then AC in the file, Skin Like Steel AC then HP),
    // so "first slot wins" off the file would reshuffle kinds across the
    // whole HP/AC family and change every row and mute key with it. A
    // prose kind the file backs with that SPA stands; one the file does
    // not back is wrong (Squire's "Add Proc" -- no SPA 85, an SPA 121)
    // and yields to the file; none at all takes the file's first.
    if let Some(k) = text {
        if spas.iter().any(|&spa| kind_of_spa(spa) == Some(k)) {
            return text;
        }
    }
    spas.iter().find_map(|&spa| kind_of_spa(spa)).or(text)
}

/// why: five minutes. Measured on the real file across every buff the
/// tracker could admit: everything it should track is 10 min or more
/// (Alacrity 110 ticks, Clarity 270, Spirit of Wolf 360), one real band
/// sits at 50-60 (Berserker Spirit, Rampage, Avatar, Impart Strength --
/// cast out of combat, re-cast on a timer), and under 50 it is heals,
/// songs, mez, roots and debuffs. 50 keeps that band and drops the rest.
const MIN_BUFF_TICKS: u32 = 50;
/// why: the cap -- a buff's length grows with level, and "is this a
/// maintained buff" should not depend on who is casting it today
const MAINTAINED_AT_LEVEL: u32 = 50;

/// why: the SPA ids the tracker has a kind for -- classic-EQ effect ids,
/// each verified on the real file (Clarity 15, Celerity 11, Shield of
/// Words 1, Aegolism 69, Berserker Spirit 4, Rune I 55, Vampiric Embrace
/// 85, Blessing of the Squire 121, Regeneration 0 with a duration)
fn kind_of_spa(spa: u16) -> Option<BuffKind> {
    Some(match spa {
        0 => BuffKind::HpRegen,
        1 => BuffKind::Ac,
        2 => BuffKind::Attack,
        3 => BuffKind::Movement,
        4 => BuffKind::Strength,
        5 => BuffKind::Dexterity,
        6 => BuffKind::Agility,
        7 => BuffKind::Stamina,
        11 => BuffKind::Haste,
        15 => BuffKind::ManaRegen,
        46..=50 => BuffKind::Resist,
        // why: 55 absorbs any damage, 78 magic damage only -- both runes
        // (Niv's Melody of Preservation carries the 78 kind)
        55 | 78 => BuffKind::Rune,
        59 => BuffKind::DamageShield,
        69 => BuffKind::Hp,
        85 => BuffKind::Proc,
        121 => BuffKind::HealOnHit,
        _ => return None,
    })
}

/// why: the wiki prose walk -- the fallback, and what the tests pin
pub fn kind_of(spell: &Spell) -> Option<BuffKind> {
    // why: checked before the slot walk -- a familiar's slots describe
    // what it GIVES (resists, mana, see invisible), so walking them files
    // it under whichever effect comes first and loses the thing itself
    if spell.name.ends_with("Familiar") {
        return Some(BuffKind::Familiar);
    }
    for slot in &spell.slots {
        let e = slot.effect.as_str();
        // why: checked before the Increase/Absorb gate -- a proc slot
        // states neither, which is why this kind did not exist
        if e.starts_with("Add Proc") {
            return Some(BuffKind::Proc);
        }
        if !e.starts_with("Increase") && !e.starts_with("Absorb") && !e.starts_with("Damage Shield")
        {
            continue;
        }
        // why: REGEN ticks. "Increase Mana by 161" is Harvest -- an
        // instant mana return with a Stun rider, a combat ability you
        // fire, not a buff you keep up ("harvest is more a mana drain, a
        // combat ability. not really a mana regen buff"). Same test the
        // HpRegen arm below already applies. Catches Cannibalize and the
        // Gift of Brilliance line too, and keeps the bard Clarity songs,
        // which do tick despite an Instant duration.
        if e.starts_with("Absorb") || e.starts_with("Increase Absorb") {
            return Some(BuffKind::Rune);
        }
        if e.contains("Mana") && e.to_ascii_lowercase().contains("per tick") {
            return Some(BuffKind::ManaRegen);
        }
        if e.contains("Attack Speed") {
            return Some(BuffKind::Haste);
        }
        if e.contains("Max Hitpoints") || e.contains("Max Hit Points") {
            return Some(BuffKind::Hp);
        }
        if e.contains("Hitpoints per Tick")
            || e.contains("Hitpoints per tick")
            || e.contains("HP per Tick")
            || e.contains("Hit Points per Tick")
        {
            return Some(BuffKind::HpRegen);
        }
        if e.starts_with("Increase AC") {
            return Some(BuffKind::Ac);
        }
        if e.contains("Resist") {
            return Some(BuffKind::Resist);
        }
        if e.contains("Movement Speed") {
            return Some(BuffKind::Movement);
        }
        if e.starts_with("Increase ATK") {
            return Some(BuffKind::Attack);
        }
        if e.starts_with("Increase STR") {
            return Some(BuffKind::Strength);
        }
        if e.starts_with("Increase DEX") {
            return Some(BuffKind::Dexterity);
        }
        if e.starts_with("Increase STA") {
            return Some(BuffKind::Stamina);
        }
        if e.starts_with("Increase AGI") {
            return Some(BuffKind::Agility);
        }
        if e.starts_with("Damage Shield") || e.starts_with("Absorb") {
            return Some(BuffKind::DamageShield);
        }
    }
    None
}

/// why: a buff on ANOTHER player -- the beneficial types, cast on a
/// friend or the group, never Self/Pet
/// why: a buff that only ever lands on its caster -- "Vampiric Embrace",
/// "Grim Aura". Not a party buff by definition (nobody can put it on
/// you), but you can put it on yourself, so it is a real missing buff
/// when your own trio has it and it is not up.
/// why: a RECOURSE is the effect half of a spell you cast on an ENEMY --
/// Spencer: "Siphon Strength is more a combat ability in the sense you
/// debuff to get it, not a 'buff' you put on". You never cast it, so it
/// is never something you are missing. EQ's own naming convention, not a
/// per-spell list: three in the pack, and two name a real parent spell.
/// One of them ("siphon strength recourse") is scraped WITH a class,
/// which is why it reached the tracker at all.
fn is_recourse(spell: &Spell) -> bool {
    spell.name.to_ascii_lowercase().ends_with(" recourse")
}

pub fn is_self_buff(spell: &Spell) -> bool {
    let beneficial = matches!(
        spell.spell_type.as_deref(),
        Some("Beneficial") | Some("Statistic Buff") | Some("Resist Buff") | Some("Movement Buff")
    );
    beneficial && spell.target_type.as_deref() == Some("Self") && !spell.classes.is_empty()
}

pub fn is_party_buff(spell: &Spell) -> bool {
    let beneficial = matches!(
        spell.spell_type.as_deref(),
        Some("Beneficial") | Some("Statistic Buff") | Some("Resist Buff") | Some("Movement Buff")
    );
    let target = spell.target_type.as_deref().unwrap_or("");
    let on_others =
        target.starts_with("Single Friendly") || target.starts_with("Group") || target == "Single";
    beneficial && on_others && !spell.classes.is_empty()
}

const MANA_USERS: &[&str] = &[
    "Enchanter",
    "Wizard",
    "Magician",
    "Necromancer",
    "Cleric",
    "Druid",
    "Shaman",
    "Paladin",
    "Shadow Knight",
    "Ranger",
    "Bard",
    "Beastlord",
];
const PURE_CASTERS: &[&str] = &["Enchanter", "Wizard", "Magician", "Necromancer"];

/// why: the three shapes a class plays as, for weighing how much a buff
/// kind is worth to it -- a pure caster wants mana, a melee wants haste,
/// a priest wants both
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    PureCaster,
    Priest,
    Melee,
}

const PRIESTS: &[&str] = &["Cleric", "Druid", "Shaman"];

fn shape_of(class: &str) -> Shape {
    if PURE_CASTERS.contains(&class) {
        Shape::PureCaster
    } else if PRIESTS.contains(&class) {
        Shape::Priest
    } else {
        Shape::Melee
    }
}

/// why: Spencer -- "rank by how relevant it is to my class". A kind's
/// worth is the most any one of your three classes gets out of it, so a
/// trio with a melee in it still ranks haste high while its caster half
/// ranks mana regen high. Numbers are an ordering, not a simulation.
pub fn relevance(kind: BuffKind, my_classes: &[String]) -> u32 {
    if !benefits(kind, my_classes) {
        return 0;
    }
    let weight = |shape: Shape| -> u32 {
        match (kind, shape) {
            (BuffKind::ManaRegen, Shape::PureCaster) => 100,
            (BuffKind::ManaRegen, Shape::Priest) => 95,
            (BuffKind::ManaRegen, Shape::Melee) => 55,
            // why: a proc rides melee swings, so it is worth what your
            // swinging half is worth -- and nothing to a pure caster
            // why: a rune soaks damage whatever you are
            (BuffKind::Rune, _) => 60,
            // why: a familiar is mana and resists that never falls off
            (BuffKind::Familiar, _) => 65,
            (BuffKind::Proc, Shape::Melee) => 70,
            (BuffKind::Proc, _) => 10,
            (BuffKind::HealOnHit, Shape::Melee) => 65,
            (BuffKind::HealOnHit, _) => 10,
            (BuffKind::Haste, Shape::Melee) => 100,
            (BuffKind::Haste, _) => 20,
            (BuffKind::Hp, Shape::Melee) => 85,
            (BuffKind::Hp, _) => 70,
            (BuffKind::Ac, Shape::Melee) => 80,
            (BuffKind::Ac, _) => 55,
            (BuffKind::Attack, Shape::Melee) => 75,
            (BuffKind::Attack, _) => 15,
            (BuffKind::Strength, Shape::Melee) => 70,
            (BuffKind::Strength, _) => 15,
            (BuffKind::HpRegen, Shape::Melee) => 60,
            (BuffKind::HpRegen, _) => 50,
            (BuffKind::Resist, _) => 50,
            (BuffKind::DamageShield, Shape::Melee) => 55,
            (BuffKind::DamageShield, _) => 30,
            (BuffKind::Dexterity, Shape::Melee) => 45,
            (BuffKind::Dexterity, _) => 15,
            (BuffKind::Stamina, Shape::Melee) => 40,
            (BuffKind::Stamina, _) => 30,
            (BuffKind::Agility, Shape::Melee) => 35,
            (BuffKind::Agility, _) => 25,
            (BuffKind::Movement, _) => 25,
        }
    };
    // why: summed over your three, not the max of them -- a kind two of
    // your classes want beats one only the third wants (ENC/SHD/WIZ:
    // mana regen 255 over haste 140), which is what "relevant to my
    // class combo" means
    my_classes.iter().map(|c| weight(shape_of(c))).sum()
}

/// why: which kinds YOUR combo gets anything out of -- mana regen needs
/// a mana user, haste/attack/STR/DEX a melee class; the rest help everyone
pub fn benefits(kind: BuffKind, my_classes: &[String]) -> bool {
    let uses_mana = my_classes.iter().any(|c| MANA_USERS.contains(&c.as_str()));
    let melees = my_classes
        .iter()
        .any(|c| !PURE_CASTERS.contains(&c.as_str()));
    match kind {
        BuffKind::ManaRegen => uses_mana,
        // why: a weapon proc is worth nothing to a trio that never swings
        BuffKind::Haste
        | BuffKind::Attack
        | BuffKind::Strength
        | BuffKind::Dexterity
        | BuffKind::Proc
        | BuffKind::HealOnHit => melees,
        _ => true,
    }
}

/// why: a buff the party cannot possibly have. Two independent cuts,
/// and the pack says neither subsumes the other: filtering on level alone
/// still admits 34 out-of-era spells at or under the cap, and filtering on
/// era alone still admits 98 era-unknown spells above it. The reported
/// case was both at once -- "Skin of the Shadow" is Kunark Era AND
/// Necromancer 55, recommended as an upgrade over Shield of Words.
///
/// An era the scrape never stated PASSES: 196 such spells sit at or under
/// the cap and are ordinary Classic buffs, so refusing them would hide
/// real recommendations to catch a few. The cap is what covers that gap.
fn reachable(spell: &Spell, ceiling: Option<usize>) -> bool {
    // why: the wiki says outright which spells no player casts. "Barrier
    // of Force isnt in the game i think? so that should be gone gone" --
    // it is categorised NPC Only, mana 0, no obtain path, and its Wizard
    // entry carries no level, so every other gate here waved it through.
    // 350 spells carry the category and only 13 still name a class, most
    // of those being mobs ("a diseased rat", "Spirit of the Puma").
    if spell
        .categories
        .iter()
        .any(|c| c.eq_ignore_ascii_case("NPC Only Spells"))
    {
        return false;
    }
    match spell.era.as_deref().and_then(crate::gearplanner::era_ix) {
        Some(ix) => ceiling.is_none_or(|l| ix <= l),
        None => true,
    }
}

/// why: one spell LINE the party could put on you -- ranks of a line are
/// one entry (Clarity I/II/III is "Clarity"), because casting any of them
/// covers the same slot
#[derive(Debug, Clone, Serialize)]
pub struct BuffLineDto {
    /// why: the line as named, rank numeral stripped
    pub line: String,
    /// why: the highest-level rank of it the party can actually cast --
    /// "usable" means at or under the caster's own level, their /who
    /// level if one printed, else the level their casts imply
    pub best_spell: String,
    /// why: the level requirement of that rank, for comparing what is
    /// actually on you against what the party could put there
    pub best_level: u32,
    pub casters: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BuffRowDto {
    pub kind: BuffKind,
    pub label: &'static str,
    /// why: on you right now -- the spell's name when it is
    pub active: Option<String>,
    /// why: the level of the rank currently on you -- a low-tier buff
    /// with a better rank available reads as an upgrade, not as covered
    /// (Spencer: "you should not have breeze on when a 30ish one should
    /// be available")
    pub active_level: Option<u32>,
    pub upgrade: bool,
    /// why: how much this kind is worth to your own classes -- rows are
    /// ordered by it, most relevant first (see `relevance`)
    pub relevance: u32,
    /// why: every line of this kind the party could cast, best first --
    /// what is assumed missing when nothing of the kind is on you
    pub lines: Vec<BuffLineDto>,
    /// why: somebody who is not YOU can cast the BEST line here. You are
    /// a source like any groupmate (a buff your own trio can cast is one
    /// you are missing), so a row can name only yourself -- and "others
    /// missing" said of a line nobody but you can cast is a lie. Judged
    /// on the best line alone, not any line: a groupmate who can only
    /// cast a worse rank of the same kind does not make the rank you
    /// should actually have their job. Read after muting.
    pub others: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PartyMemberDto {
    pub name: String,
    pub classes: Vec<String>,
    pub level: Option<u8>,
    pub confirmed: bool,
    /// why: what is on THEM right now, by kind label -- read from the
    /// landings your log saw on them, each kept for its own duration.
    /// Only what your log witnessed; a buff cast before you grouped is
    /// invisible, so this under-reports rather than inventing coverage.
    pub buffs: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GroupBuffsDto {
    /// why: every expected kind covered by the best rank the party can
    /// cast -- the one word the widget leads with
    pub good: bool,
    /// why: kinds that ARE up, but from a lower rank than the party could cast
    pub upgrades: usize,
    pub my_classes: Vec<String>,
    pub party: Vec<PartyMemberDto>,
    pub rows: Vec<BuffRowDto>,
    /// why: your OWN self-casts, as a flat checklist -- Grim Aura,
    /// Vampiric Embrace. Not lines with upgrades; just on or not.
    pub innates: Vec<SelfBuffDto>,
    /// why: illusions -- real stats, but a suggestion rather than
    /// something missing. Shown quieter, and never counted against you.
    pub maybes: Vec<SelfBuffDto>,
    /// why: every line name the tracker knows about right now, muted
    /// ones included -- what Overlay settings lists to mute from
    pub catalog: Vec<String>,
    /// why: buffs on you the party can't account for (someone outside
    /// the group, or an unknown class) -- listed, never counted against
    pub extra_active: Vec<String>,
}

/// why: "Clarity II" and "Clarity" are one line -- a trailing roman
/// numeral is a rank, same rule the spellbook's own line grouping uses
fn base_name(name: &str) -> &str {
    match name.rsplit_once(' ') {
        Some((base, tail))
            if !tail.is_empty()
                && tail
                    .bytes()
                    .all(|b| matches!(b, b'I' | b'V' | b'X' | b'L' | b'C' | b'D' | b'M')) =>
        {
            base
        }
        _ => name,
    }
}

/// why: the line a spell belongs to. Real data (2026-09-03): "Clarity"
/// and "Clarity II" carry different descriptions (the higher rank
/// appends a forum link and a min-level note), so only the rank-stripped
/// name groups them. Renamed tiers of one family (Breeze, Clarity) stay
/// separate lines on purpose -- either one covers the slot, and naming
/// both tells you what the party could actually cast.
fn line_key(spell: &Spell) -> String {
    base_name(&spell.name).to_string()
}

/// why: EQ names a rank line two ways -- a numeral ("Clarity II", which
/// `base_name` already strips) or a leading rank word ("Minor", "Lesser",
/// "Major", "Greater", "Arch"). Only the second vocabulary groups a line;
/// grouping on the trailing word instead would merge Grim Aura with
/// Divine Aura, Banshee Aura and Null Aura, which are different spells
/// that happen to end the same way -- one of them the Cleric invuln.
///
/// Reported as "Dark Temptation and Grim Aura fit the same bucket, but
/// they stack": keyed by BuffKind they were one entry, so whichever was
/// up hid the other. Nothing in the packs says which buffs stack --
/// spell_stacking.json is 48 entries of poisons and DoTs, and neither of
/// those two is in it -- so the honest unit is the LINE, and two spells
/// that are not ranks of each other are two lines.
const RANK_WORDS: &[&str] = &[
    "minor", "lesser", "greater", "major", "arch", "superior", "improved",
];

fn rank_line(name: &str) -> String {
    let stripped = base_name(name);
    match stripped.split_once(' ') {
        Some((head, rest)) if RANK_WORDS.iter().any(|w| head.eq_ignore_ascii_case(w)) => {
            rest.to_string()
        }
        _ => stripped.to_string(),
    }
}

/// why: a buff is only an upgrade over a DIFFERENT buff. Reported live as
/// "Vampiric Embrace -> Vampiric Embrace": the two sides measure level
/// differently and always have -- the active side takes the MINIMUM class
/// requirement across the spell's classes (Vampiric Embrace is Necromancer
/// 7, Shadow Knight 15, so 7), the best side takes the requirement for the
/// class the CASTER actually has (15 for a Shadow Knight). Same spell,
/// 7 < 15, "upgrade". Comparing the names settles it without disturbing
/// either number, both of which are right for what they are used for
/// elsewhere -- castability, and the level shown on the row.
fn is_upgrade(active: Option<(&str, u32)>, best_spell: Option<&str>, best_level: u32) -> bool {
    let Some((name, level)) = active else {
        return false;
    };
    let Some(best) = best_spell else {
        return level < best_level;
    };
    if best.eq_ignore_ascii_case(name) {
        return false;
    }
    // why: the same value model the lists are ordered by -- a hand-ranked
    // line keeps its rank here too. Reported as "weapon proc: Vampiric
    // Embrace -> Blessing of the Squire": VE (7) lost to the cleric proc
    // line (16) on raw level, the exact comparison the override exists to
    // correct. A Shadow Knight wants VE over the cleric line, allies or not.
    value_of(base_name(name), level) < value_of(base_name(best), best_level)
}

/// why: level is a decent proxy for power and stays the default -- a
/// higher-level rank of a line is nearly always the better one. A few
/// lines are worth more than their level says, and nothing in the packs
/// carries that number: "Add Proc: VampEmbraceNecro" is all the slot
/// text says about a permanent lifetap proc. Reported as "vampiric
/// embrace is being put below cleric spell line for similar skills. yet
/// i think vampiric is highest true value". Hand-listed on purpose, one
/// entry per line that needs it, keyed by the same `rank_line` name the
/// rows and innates are grouped under. Documented in
/// docs/class-and-level-rules.md.
const VALUE_OVERRIDE: &[(&str, u32)] = &[
    ("Vampiric Embrace", 60),
    // why: Spencer -- "Clarity is better than boon of the clear mind
    // in the mana regen line, despite being lower level". ENC 26
    // against ENC 42, and level is the only thing the packs rank by.
    ("Clarity", 60),
];

/// why: the sort key for "which of these is better" -- the override when
/// a line has one, its level requirement otherwise
fn value_of(line: &str, level: u32) -> u32 {
    VALUE_OVERRIDE
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(line))
        .map_or(level, |(_, v)| *v)
}

/// why: an illusion is a MAYBE -- it grants real stats, but "put on a
/// wolf form for the ATK" is a suggestion, not a checklist item. Read off
/// the slot, not the name: 47 spells carry an Illusion effect against 26
/// called "Illusion: ...", and the difference is Call of Bones, Form of
/// Bleached Bone and friends.
fn is_illusion(spell: &Spell) -> bool {
    spell
        .slots
        .iter()
        .any(|slot| slot.effect.starts_with("Illusion"))
}

/// why: one self-cast buff line of your own. No upgrade arrow and no
/// caster list -- nobody else is involved, so the only question is
/// whether it is on. Spencer: "grim aura as its own track ... a list of
/// 'make sure these are on' as innates".
#[derive(Debug, Clone, Serialize)]
pub struct SelfBuffDto {
    /// why: rank numeral stripped, same line grouping the party rows use
    pub line: String,
    /// why: the STAT it fills, not the spell -- "weapon proc", "attack".
    /// Spencer: "lists out the assosciated buff/buff stat (not the spell
    /// but the slot it fills)". Empty only if the line's own best spell
    /// left the catalog, which cannot happen for a line built from it.
    pub label: &'static str,
    /// why: the best rank of it YOU can cast
    pub best_spell: String,
    pub best_level: u32,
    /// why: what of this line is on you right now, if anything
    pub active: Option<String>,
}

/// why: `muted` -- line names the player switched off in Overlay
/// settings. Filtered here rather than in the widget so the verdict
/// ("All good") and the counts agree with what is actually on screen.
pub fn group_buffs(ing: &Ingest, muted: &[String], ceiling: Option<usize>) -> GroupBuffsDto {
    let now = ing.now_ms();
    let my_classes: Vec<String> = ing
        .store
        .names
        .get("You")
        .map(|y| {
            let by_visit = ing.classes.configuration_of_visit(y.0, ing.unit_at(now));
            if by_visit.is_empty() {
                ing.classes
                    .configurations_of(y.0)
                    .into_iter()
                    .next()
                    .map(|(c, _)| c)
                    .unwrap_or_default()
            } else {
                by_visit
            }
        })
        .unwrap_or_default();

    let mut party: Vec<PartyMemberDto> = Vec::new();
    for (key, _, _, _) in ing.groups.current_members(now) {
        // why: the roster keys by fold, not display casing -- resolve it
        // the way every other reader does, or the class chain and the
        // effect pings (both keyed by the interned name) come back empty
        let name = ing.encounters.entities.display_name(&key).to_string();
        if name.eq_ignore_ascii_case("You") || ing.effective_kind(&name, now) == Kind::Pet {
            continue;
        }
        // why: one class model -- a /who row is ground truth (trio and
        // level), else the chain's own confirmed classes; a class still
        // short of the bar is a guess and does not decide a buff
        let (level, _from_who) = ing.ally_level(&name, now);
        let (classes, confirmed) = match ing.class_chain(&name, now) {
            Some(view) => match view.who.clone() {
                Some((_, trio)) => (trio, true),
                None => (view.confirmed.clone(), !view.confirmed.is_empty()),
            },
            None => (Vec::new(), false),
        };
        let buffs = buffs_on(ing, &name, now)
            .into_iter()
            .map(|k| k.label().to_string())
            .collect();
        party.push(PartyMemberDto {
            name,
            classes,
            level,
            confirmed,
            buffs,
        });
    }

    // why: YOU are a source too -- asked for directly ("should use
    // detected innate buffs for current classes"). The roster above
    // deliberately skips you (it answers "who am I grouped with"), but a
    // buff your own detected trio can cast is one you are missing when it
    // is not up, and nobody else has to be online for it. Kept out of
    // `party` so the roster line still reads as the group.
    let me = PartyMemberDto {
        name: "You".to_string(),
        classes: my_classes.clone(),
        level: ing.ally_level("You", now).0,
        confirmed: !my_classes.is_empty(),
        buffs: Vec::new(),
    };

    // why: per kind, the best (highest-level) castable spell and everyone
    // who could cast it -- confirmed classes only
    type LineAcc = BTreeMap<String, (u32, String, HashSet<String>)>;
    let mut best: BTreeMap<BuffKind, LineAcc> = BTreeMap::new();
    let sources = party
        .iter()
        .map(|m| (false, m))
        .chain(std::iter::once((true, &me)));
    for (is_me, member) in sources.filter(|(_, m)| m.confirmed) {
        for spell in spells() {
            // why: a self-only buff is not a party buff -- nobody can put
            // it on you -- but you can put it on YOURSELF, so it is a real
            // missing buff when your own trio has it and it is not up
            // why: self-casts are no longer folded in here -- they became
            // their own `innates`/`maybes` sections, because an upgrade
            // arrow between a party buff and a thing only you can cast on
            // yourself was never a real relationship
            let _ = is_me;
            if is_recourse(spell) || !is_party_buff(spell) {
                continue;
            }
            let Some(kind) = kind_of_with(spell, ing.spell_file()) else {
                continue;
            };
            // why: `benefits` stops a groupmate offering mana regen to a
            // pure melee. It cannot arise for a spell you cast on
            // YOURSELF -- your own class having it settles the question.
            if !is_me && !benefits(kind, &my_classes) {
                continue;
            }
            if !reachable(spell, ceiling) {
                continue;
            }
            let castable = spell.classes.iter().find(|sc| {
                member.classes.iter().any(|c| c == &sc.class)
                    // why: nobody can reach a rank above the server's cap,
                    // whatever their own level is or isn't -- the spell
                    // scrape carries Live's levels (Improved Invisibility
                    // is listed Wizard 55), and an unknown ally level used
                    // to let every one of those through
                    && sc.level.is_none_or(|need| need <= u32::from(LEVEL_CAP))
                    && member
                        .level
                        .is_none_or(|lvl| sc.level.is_none_or(|need| need <= u32::from(lvl)))
            });
            let Some(sc) = castable else { continue };
            let rank = sc.level.unwrap_or(0);
            let e = best
                .entry(kind)
                .or_default()
                .entry(line_key(spell))
                .or_insert_with(|| (0, spell.name.clone(), HashSet::new()));
            // why: within a line, the highest rank anyone can cast is the
            // one worth naming; every member who can cast any rank of it
            // is a caster for the line
            if rank >= e.0 {
                e.0 = rank;
                e.1 = spell.name.clone();
            }
            e.2.insert(member.name.clone());
        }
    }

    // why: what's on you, by kind
    let mut active_by_kind: HashMap<BuffKind, (String, u32)> = HashMap::new();
    let mut extra_active: Vec<String> = Vec::new();
    for (spell_name, (_, expires)) in &ing.self_buffs {
        if expires.is_some_and(|e| e < now) {
            continue;
        }
        let Some(spell) = crate::spelldata::spell_by_name(spell_name) else {
            continue;
        };
        // why: an illusion never stands in for a party buff. Reported as
        // "strength: Illusion: Earth Elemental -> Berserker Spirit" -- the
        // form was being read as the buff on you and Berserker Spirit as
        // an upgrade over it, when the honest statement is just that
        // Berserker Spirit is not up. Forms answer maybes, nothing else.
        if is_illusion(spell) {
            continue;
        }
        match kind_of_with(spell, ing.spell_file()) {
            Some(k) => {
                // why: the rank ON you, by its own level requirement --
                // the highest one up of that kind is what counts
                let lvl = spell
                    .classes
                    .iter()
                    .filter_map(|c| c.level)
                    .min()
                    .unwrap_or(0);
                let e = active_by_kind
                    .entry(k)
                    .or_insert_with(|| (spell_name.clone(), lvl));
                if lvl > e.1 {
                    *e = (spell_name.clone(), lvl);
                }
            }
            None => extra_active.push(spell_name.clone()),
        }
    }

    // why: your OWN self-casts, split from the party rows -- a flat
    // checklist ("make sure these are on") and, separately, illusions as
    // suggestions. Best rank per line, castable at your own level.
    let mut innate_by_line: BTreeMap<String, (u32, String, bool)> = BTreeMap::new();
    if !my_classes.is_empty() {
        for spell in spells() {
            // why: kind_of is the filter that makes this a BUFF list --
            // it admits the real stat buffs on their own effects and
            // rejects the summon-item, enchant-metal, water-breathing and
            // ultravision spells that are also self-target and beneficial.
            // Without it this ran to 92 entries, most of them not buffs.
            if !is_self_buff(spell)
                || is_recourse(spell)
                || !reachable(spell, ceiling)
                || kind_of_with(spell, ing.spell_file()).is_none()
            {
                continue;
            }
            let Some(sc) = spell.classes.iter().find(|sc| {
                my_classes.iter().any(|c| c == &sc.class)
                    && sc.level.is_none_or(|need| need <= u32::from(LEVEL_CAP))
                    && me
                        .level
                        .is_none_or(|lvl| sc.level.is_none_or(|need| need <= u32::from(lvl)))
            }) else {
                continue;
            };
            let rank = sc.level.unwrap_or(0);
            // why: keyed by KIND, not by line. EQ's self-buff lines rename
            // per rank (Minor/Lesser/Major/Greater/Arch Shielding), which
            // `line_key` cannot group because it only strips numerals --
            // keyed by line the checklist showed six Shieldings. One entry
            // per thing you want up is what a checklist means. Illusions
            // key separately so a wolf form never hides a real AC buff.
            let illusion = is_illusion(spell);
            // why: keyed by LINE, not by kind. Two ATK self-buffs that
            // STACK (Grim Aura and Dark Temptation) are two things to keep
            // up, not one bucket where whichever is on hides the other.
            // The rank vocabulary still collapses a real line, so the six
            // Shieldings stay one entry. Illusions key together, since you
            // wear one form at a time.
            let key = if illusion {
                "~illusion".to_string()
            } else {
                rank_line(&spell.name)
            };
            let e = innate_by_line
                .entry(key)
                .or_insert_with(|| (0, spell.name.clone(), illusion));
            if rank >= e.0 {
                *e = (rank, spell.name.clone(), illusion);
            }
        }
    }
    // why: an innate is on when one of YOUR OWN self-casts of that kind is
    // on -- not when a groupmate's buff happens to cover the same kind.
    // `active_by_kind` answers the latter, and using it read "armor class:
    // covered" off a party AC buff while your own Shielding was down.
    // A lower rank of your own still counts.
    let active_self_by_line: HashMap<String, String> = ing
        .self_buffs
        .iter()
        .filter(|(_, (_, expires))| !expires.is_some_and(|e| e < now))
        .filter_map(|(name, _)| {
            let sp = crate::spelldata::spell_by_name(name)?;
            // why: an ILLUSION never satisfies an innate. Wearing a Dry
            // Bone form covers the maybe, not the "keep Lesser Familiar
            // up" item -- different asks, and letting a form tick the
            // checklist hid real missing self-buffs.
            (is_self_buff(sp) && !is_recourse(sp) && !is_illusion(sp)).then_some(())?;
            // why: by LINE -- a lower rank of the same line still counts
            // as on, and two stacking spells stay two answers
            Some((rank_line(&sp.name), name.clone()))
        })
        .collect();
    // why: you can only wear one form at a time, so a single active
    // illusion answers for every illusion suggestion -- "if an illusion is
    // active, discount all other illusion suggestions". Kind-by-kind was
    // wrong here: wearing a wolf covers the ATK maybe but left the Dry
    // Bone and Earth Elemental ones still asking to be cast.
    let active_illusion: Option<String> = ing
        .self_buffs
        .iter()
        .filter(|(_, (_, expires))| !expires.is_some_and(|e| e < now))
        .find_map(|(name, _)| {
            let sp = crate::spelldata::spell_by_name(name)?;
            is_illusion(sp).then(|| name.clone())
        });
    let (mut innates, mut maybes) = (Vec::new(), Vec::new());
    for (key, (best_level, best_spell, illusion)) in innate_by_line {
        let line = if illusion {
            "illusion".to_string()
        } else {
            key.clone()
        };
        let active = if illusion {
            // why: any illusion covers them all
            active_illusion.clone()
        } else if crate::spelldata::spell_by_name(&best_spell).and_then(kind_of)
            == Some(BuffKind::Familiar)
        {
            // why: a familiar lands no buff message -- "You summon forth a
            // lesser familiar." is the only confirmation it is out
            ing.familiar_since_ms.map(|_| best_spell.clone())
        } else {
            active_self_by_line.get(&line).cloned()
        };
        let label = crate::spelldata::spell_by_name(&best_spell)
            .and_then(kind_of)
            .map_or("", |k| k.label());
        let dto = SelfBuffDto {
            active,
            line,
            label,
            best_spell,
            best_level,
        };
        if illusion {
            maybes.push(dto);
        } else {
            innates.push(dto);
        }
    }
    // why: most valuable first -- level requirement unless the line
    // carries an override (see VALUE_OVERRIDE)
    let by_value = |a: &SelfBuffDto, b: &SelfBuffDto| {
        value_of(&b.line, b.best_level)
            .cmp(&value_of(&a.line, a.best_level))
            .then(a.line.cmp(&b.line))
    };
    innates.sort_by(by_value);
    maybes.sort_by(by_value);

    let mut rows: Vec<BuffRowDto> = best
        .into_iter()
        .map(|(kind, by_line)| {
            let mut lines: Vec<(u32, BuffLineDto)> = by_line
                .into_values()
                .map(|(rank, best_spell, casters)| {
                    let mut casters: Vec<String> = casters.into_iter().collect();
                    casters.sort();
                    (
                        rank,
                        BuffLineDto {
                            line: base_name(&best_spell).to_string(),
                            best_spell,
                            best_level: rank,
                            casters,
                        },
                    )
                })
                .collect();
            // why: same value model the innates use -- the level
            // requirement unless the line carries an override
            lines.sort_by(|a, b| {
                value_of(&b.1.line, b.0)
                    .cmp(&value_of(&a.1.line, a.0))
                    .then_with(|| a.1.line.cmp(&b.1.line))
            });
            let best_level = lines.first().map(|(r, _)| *r).unwrap_or(0);
            let on_you = active_by_kind.get(&kind).cloned();
            let active_level = on_you.as_ref().map(|(_, l)| *l);
            // why: covered means the BEST usable rank is up -- Breeze
            // while the party's Enchanter can cast Clarity is an upgrade.
            //
            // The SAME spell never is. Reported as "Vampiric Embrace ->
            // Vampiric Embrace": the two sides measure level differently
            // and always have -- the active side takes the MINIMUM class
            // requirement across the spell's classes (Vampiric Embrace is
            // Necromancer 7, Shadow Knight 15, so 7), while the best side
            // takes the requirement for the class the CASTER actually has
            // (15 for a Shadow Knight). Same spell, 7 < 15, "upgrade".
            // Comparing the names settles it without disturbing either
            // number, both of which are right for what they are used for
            // elsewhere -- castability, and the level shown on the row.
            let upgrade = is_upgrade(
                on_you.as_ref().map(|(n, l)| (n.as_str(), *l)),
                lines.first().map(|(_, l)| l.best_spell.as_str()),
                best_level,
            );
            BuffRowDto {
                kind,
                label: kind.label(),
                active: on_you.map(|(n, _)| n),
                active_level,
                upgrade,
                relevance: relevance(kind, &my_classes),
                lines: lines.into_iter().map(|(_, l)| l).collect(),
                // why: set below, once muting has settled which lines
                // are still on the row
                others: false,
            }
        })
        .collect();
    // why: most relevant to your own classes first -- what is missing at
    // the top is what actually costs you
    rows.sort_by(|a, b| {
        b.relevance
            .cmp(&a.relevance)
            .then_with(|| a.label.cmp(b.label))
    });
    // why: every line the tracker considered, muted or not -- Overlay
    // settings lists this to offer the mute toggles, so a line has to
    // stay listable after it is switched off
    let mut catalog: Vec<String> = rows
        .iter()
        .flat_map(|r| r.lines.iter().map(|l| l.line.clone()))
        .chain(innates.iter().map(|i| i.line.clone()))
        .chain(maybes.iter().map(|m| m.line.clone()))
        .collect();
    catalog.sort();
    catalog.dedup();
    retain_unmuted(&mut rows, &mut innates, &mut maybes, muted);
    for r in &mut rows {
        r.others = cast_by_others(&r.lines);
    }
    let upgrades = rows.iter().filter(|r| r.upgrade).count();
    // why: an innate you can cast and have not is missing the same way a
    // party buff is. A MAYBE never counts against you -- that is what
    // makes it a maybe.
    let good = rows.iter().all(|r| r.active.is_some())
        && innates.iter().all(|i| i.active.is_some())
        && upgrades == 0;
    extra_active.sort();
    GroupBuffsDto {
        good,
        upgrades,
        my_classes,
        party,
        rows,
        innates,
        maybes,
        catalog,
        extra_active,
    }
}

/// why: whose job the buff is. Judged on the BEST line alone: a groupmate
/// who can only cast a worse rank of the same kind does not make the rank
/// you should actually have their job. And a line YOU can cast is never
/// theirs -- Spencer: "if the self can cast it, dont offset it to group
/// to cast, like if enc buff and enc is in trio, dont say group should
/// cast it when I can cast it". So a row you can cover yourself reads as
/// your own shortfall even when a groupmate could also cast it.
fn cast_by_others(lines: &[BuffLineDto]) -> bool {
    lines.first().is_some_and(|l| {
        !l.casters.iter().any(|c| c.eq_ignore_ascii_case("You"))
            && l.casters.iter().any(|c| !c.eq_ignore_ascii_case("You"))
    })
}

/// why: a muted line is not a suggestion and not a shortfall -- it
/// leaves the rows, the checklist and the maybes together, before the
/// verdict is counted, so "All good" cannot be contradicted by something
/// the player switched off. A row whose every line is muted has nothing
/// left to say and goes with them.
fn retain_unmuted(
    rows: &mut Vec<BuffRowDto>,
    innates: &mut Vec<SelfBuffDto>,
    maybes: &mut Vec<SelfBuffDto>,
    muted: &[String],
) {
    let is_muted = |line: &str| muted.iter().any(|m| m.eq_ignore_ascii_case(line));
    for r in rows.iter_mut() {
        r.lines.retain(|l| !is_muted(&l.line));
    }
    rows.retain(|r| !r.lines.is_empty());
    innates.retain(|i| !is_muted(&i.line));
    maybes.retain(|m| !is_muted(&m.line));
}

/// why: how far back a landing on someone else can still be up -- the
/// longest real buff durations are hours, and each ping is checked
/// against its own spell's duration anyway
const OTHERS_BUFF_WINDOW_MS: Millis = 6 * 3600 * 1000;

/// why: the buff kinds currently on someone who is not you, from the
/// landings your log saw on them (`self_buffs` is the same idea for you,
/// fed by first-person text instead)
fn buffs_on(ing: &Ingest, name: &str, now: Millis) -> Vec<BuffKind> {
    let Some(sym) = ing.store.names.get(name) else {
        return Vec::new();
    };
    let mut kinds: HashSet<BuffKind> = HashSet::new();
    for ping in ing.effects.recent(sym.0, now, OTHERS_BUFF_WINDOW_MS) {
        let Some(spell) = crate::spelldata::spell_by_name(&ping.text) else {
            continue;
        };
        let Some(kind) = kind_of_with(spell, ing.spell_file()) else {
            continue;
        };
        if expiry_for(spell, ping.ts).is_none_or(|e| e >= now) {
            kinds.insert(kind);
        }
    }
    let mut v: Vec<BuffKind> = kinds.into_iter().collect();
    v.sort();
    v
}

/// why: how long a landed buff stays counted -- the catalog's duration
/// (the top of a leveled range), else an hour; a wear-off line ends it early
pub fn expiry_for(spell: &Spell, landed_at: Millis) -> Option<Millis> {
    let d = crate::spelleffect::parse_duration(spell.duration.as_deref());
    if d.is_permanent {
        return None;
    }
    let secs = d.max_secs.or(d.min_secs).unwrap_or(3600.0);
    Some(landed_at + (secs * 1000.0) as Millis)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// why: "Hand rank exceptions but default higher level is better" --
    /// the override is the only thing that outranks a level, and a line
    /// without one is ordered by its level exactly as before
    #[test]
    fn a_hand_ranked_line_outranks_its_level() {
        // why: Vampiric Embrace is Shadow Knight 15; the cleric-line
        // self-casts it kept losing to sit in the 30s and 40s
        assert!(value_of("Vampiric Embrace", 15) > value_of("Shield of Words", 39));
        assert_eq!(
            value_of("Shield of Words", 39),
            39,
            "no override, no change"
        );
        assert_eq!(
            value_of("vampiric embrace", 15),
            value_of("Vampiric Embrace", 15)
        );
    }

    /// why: "Others missing" means party members who can buff you have
    /// not. A line YOU can cast is your own problem even when a groupmate
    /// could cast it too (Spencer: "if the self can cast it, dont offset
    /// it to group to cast"), and a groupmate who can only manage a worse
    /// rank of the kind does not own the rank you should actually have.
    #[test]
    fn only_the_best_lines_caster_decides_whose_job_it_is() {
        let line = |n: &str, casters: &[&str]| BuffLineDto {
            line: n.to_string(),
            best_spell: n.to_string(),
            best_level: 1,
            casters: casters.iter().map(|c| c.to_string()).collect(),
        };
        assert!(!cast_by_others(&[]), "no line, nobody's job");
        assert!(!cast_by_others(&[line("Berserker Spirit", &["You"])]));
        assert!(cast_by_others(&[line("Aegolism", &["Sorien"])]));
        assert!(
            !cast_by_others(&[line("Aegolism", &["You", "Sorien"])]),
            "you can cast it, so it is not offset to the group"
        );
        assert!(
            !cast_by_others(&[
                line("Berserker Spirit", &["You"]),
                line("Chant of Battle", &["Kaeus"]),
            ]),
            "a worse rank someone else has is not the rank you should have"
        );
    }

    /// why: a muted line leaves the tracker entirely -- if it only left
    /// the display, "All good" would still be withheld by something the
    /// player switched off
    #[test]
    fn a_muted_line_leaves_the_rows_the_innates_and_the_maybes() {
        let line = |n: &str| BuffLineDto {
            line: n.to_string(),
            best_spell: n.to_string(),
            best_level: 1,
            casters: Vec::new(),
        };
        let row = |kind, lines: Vec<BuffLineDto>| BuffRowDto {
            others: false,
            kind,
            label: kind.label(),
            active: None,
            active_level: None,
            upgrade: false,
            relevance: 0,
            lines,
        };
        let innate = |n: &str| SelfBuffDto {
            line: n.to_string(),
            label: "",
            best_spell: n.to_string(),
            best_level: 1,
            active: None,
        };
        let mut rows = vec![
            row(BuffKind::Resist, vec![line("Cure Disease")]),
            row(BuffKind::ManaRegen, vec![line("Clarity"), line("Breeze")]),
        ];
        let mut innates = vec![innate("Grim Aura")];
        let mut maybes = vec![innate("illusion")];
        retain_unmuted(
            &mut rows,
            &mut innates,
            &mut maybes,
            &[
                "cure disease".to_string(),
                "Grim Aura".to_string(),
                "illusion".to_string(),
            ],
        );
        assert_eq!(
            rows.len(),
            1,
            "a row with every line muted has nothing to say"
        );
        assert_eq!(rows[0].kind, BuffKind::ManaRegen);
        assert_eq!(rows[0].lines.len(), 2, "an unmuted sibling line stays");
        assert!(innates.is_empty());
        assert!(maybes.is_empty());
    }

    #[test]
    fn kinds_classify_the_common_buff_lines() {
        let k = |n: &str| crate::spelldata::spell_by_name(n).and_then(kind_of);
        assert_eq!(k("Clarity"), Some(BuffKind::ManaRegen));
        assert_eq!(k("Aegolism"), Some(BuffKind::Hp));
        assert_eq!(k("Spirit of Wolf"), Some(BuffKind::Movement));
    }

    /// why: Spencer -- the tracker should name the lines assumed missing.
    /// Ranks of one line are one entry; genuinely different lines are not
    /// folded together
    #[test]
    fn ranks_of_a_line_share_an_entry_and_different_lines_do_not() {
        let clarity = crate::spelldata::spell_by_name("Clarity").expect("in pack");
        let clarity_ii = crate::spelldata::spell_by_name("Clarity II").expect("in pack");
        let breeze = crate::spelldata::spell_by_name("Breeze").expect("in pack");
        assert_eq!(line_key(clarity), line_key(clarity_ii));
        assert_ne!(line_key(clarity), line_key(breeze));
        assert_eq!(line_key(clarity_ii), "Clarity");
        assert_eq!(base_name("Clarity II"), "Clarity");
        assert_eq!(base_name("Spirit of Wolf"), "Spirit of Wolf");
    }

    /// why: Spencer -- "rank by how relevant it is to my class". Your own
    /// ENC/SHD/WIZ combo puts mana regen first and still ranks haste for
    /// the Shadow Knight half; a pure melee combo inverts it
    #[test]
    fn kinds_rank_by_what_your_own_classes_get_out_of_them() {
        let mine = vec![
            "Enchanter".to_string(),
            "Shadow Knight".to_string(),
            "Wizard".to_string(),
        ];
        let r = |k| relevance(k, &mine);
        assert!(r(BuffKind::ManaRegen) > r(BuffKind::Haste));
        assert!(r(BuffKind::Haste) > 0, "the SK half melees");
        assert!(r(BuffKind::Hp) > r(BuffKind::Movement));

        let melee = vec![
            "Warrior".to_string(),
            "Rogue".to_string(),
            "Monk".to_string(),
        ];
        let m = |k| relevance(k, &melee);
        assert_eq!(m(BuffKind::ManaRegen), 0, "no mana user in the trio");
        assert!(m(BuffKind::Haste) > m(BuffKind::Hp));
        assert!(m(BuffKind::Attack) > m(BuffKind::Agility));
    }

    #[test]
    fn mana_regen_only_matters_to_a_mana_user() {
        assert!(benefits(BuffKind::ManaRegen, &["Enchanter".into()]));
        assert!(!benefits(
            BuffKind::ManaRegen,
            &["Warrior".into(), "Rogue".into()]
        ));
        assert!(benefits(BuffKind::Haste, &["Warrior".into()]));
        assert!(!benefits(
            BuffKind::Haste,
            &["Wizard".into(), "Enchanter".into()]
        ));
        assert!(benefits(BuffKind::Hp, &["Wizard".into()]));
    }

    /// why: "Dark Temptation and Grim Aura fit the same bucket, but they
    /// stack" -- keyed by BuffKind they were one entry, so whichever was
    /// up hid the other. Nothing in the packs says which buffs stack
    /// (spell_stacking.json is 48 poisons and DoTs, neither of these), so
    /// the honest unit is the LINE.
    #[test]
    fn a_rank_line_collapses_and_two_stacking_buffs_do_not() {
        // why: EQ's rank vocabulary, which is what makes a line a line
        assert_eq!(rank_line("Minor Shielding"), "Shielding");
        assert_eq!(rank_line("Arch Shielding"), "Shielding");
        assert_eq!(rank_line("Shielding"), "Shielding");
        assert_eq!(rank_line("Lesser Familiar"), "Familiar");
        assert_eq!(rank_line("Minor Familiar"), "Familiar");

        // why: two spells that stack are two answers
        assert_ne!(rank_line("Grim Aura"), rank_line("Dark Temptation"));

        // why: grouping on the TRAILING word instead would merge these,
        // and one of them is the Cleric invulnerability
        assert_ne!(rank_line("Grim Aura"), rank_line("Divine Aura"));
        assert_ne!(rank_line("Grim Aura"), rank_line("Banshee Aura"));

        // why: the numeral form still folds, as it always did
        assert_eq!(rank_line("Clarity II"), "Clarity");
    }

    /// why: "Barrier of Force isnt in the game i think? so that should be
    /// gone gone" -- it is an NPC-only spell whose Wizard entry carries no
    /// level and no mana, so the era gate, the cap gate and the class gate
    /// all waved it through. The wiki states the fact outright.
    #[test]
    fn an_npc_only_spell_is_not_reachable() {
        let bof = spells()
            .iter()
            .find(|s| s.name == "Barrier of Force")
            .expect("Barrier of Force");
        assert!(bof.categories.iter().any(|c| c == "NPC Only Spells"));
        assert!(
            bof.classes.iter().all(|c| c.level.is_none()),
            "no level, which is why the cap gate passed it"
        );
        assert!(
            bof.era.is_none(),
            "no era, which is why the era gate passed it"
        );
        assert!(
            !reachable(
                bof,
                crate::gearplanner::era_ix(crate::gearplanner::CURRENT_ERA)
            ),
            "nobody casts it"
        );

        // why: an ordinary player spell is untouched
        assert!(reachable(
            spells()
                .iter()
                .find(|s| s.name == "Clarity")
                .expect("Clarity"),
            crate::gearplanner::era_ix(crate::gearplanner::CURRENT_ERA)
        ));
        // why: the ceiling is the SETTING now, not a constant -- "All
        // eras" (no ceiling) must stop refusing anything on era grounds
        let kunark = spells()
            .iter()
            .find(|s| {
                s.era.as_deref() == Some("Kunark Era")
                    && !s
                        .categories
                        .iter()
                        .any(|c| c.eq_ignore_ascii_case("NPC Only Spells"))
            })
            .expect("a Kunark spell");
        assert!(
            !reachable(
                kunark,
                crate::gearplanner::era_ix(crate::gearplanner::CURRENT_ERA)
            ),
            "past the Sky Era ceiling"
        );
        assert!(reachable(kunark, None), "no ceiling, no era refusal");
    }

    /// why: multi-effect spells were filing under whichever effect the
    /// slot walk hit first -- Barrier of Force is a Wizard RUNE that landed
    /// under mana regen off its third slot, and Lesser Familiar landed
    /// under resists off its first. Both now have a bucket of their own.
    #[test]
    fn a_rune_and_a_familiar_are_not_what_their_first_slot_says() {
        let by = |n: &str| spells().iter().find(|s| s.name == n).expect(n);

        let rune = by("Barrier of Force");
        assert!(
            rune.slots
                .iter()
                .any(|sl| sl.effect.contains("Mana") && sl.effect.contains("per tick")),
            "it really does carry mana regen, which is why it hid there"
        );
        assert_eq!(kind_of(rune), Some(BuffKind::Rune));

        let fam = by("Lesser Familiar");
        assert!(
            fam.slots.iter().any(|sl| sl.effect.contains("Resist")),
            "its first slot really is resists, which is why it hid there"
        );
        assert_eq!(kind_of(fam), Some(BuffKind::Familiar));
        assert_eq!(kind_of(by("Minor Familiar")), Some(BuffKind::Familiar));

        // why: the real resist buff takes the resists slot back
        assert_eq!(kind_of(by("Elemental Armor")), Some(BuffKind::Resist));
    }

    /// why: an illusion is read off its SLOT, not its name -- 47 spells
    /// carry the effect against 26 called "Illusion: ...", and the
    /// difference is Call of Bones and its line. And Harvest is not mana
    /// regen: "harvest is more a mana drain, a combat ability".
    #[test]
    fn a_form_is_an_illusion_and_an_instant_drain_is_not_regen() {
        let by = |n: &str| spells().iter().find(|s| s.name == n).expect(n);

        assert!(is_illusion(by("Illusion: Spirit Wolf")));
        assert!(
            is_illusion(by("Call of Bones")),
            "an illusion that is not named like one"
        );
        assert!(!is_illusion(by("Grim Aura")), "a real innate");

        // why: regen TICKS -- Harvest returns mana once and stuns, which
        // is an ability you fire, not a buff you keep up
        let harvest = by("Harvest");
        assert_eq!(harvest.duration.as_deref(), Some("Instant"));
        assert_ne!(
            kind_of(harvest),
            Some(BuffKind::ManaRegen),
            "an instant mana return is not regen"
        );
        assert_eq!(kind_of(by("Clarity")), Some(BuffKind::ManaRegen));
        assert_eq!(
            kind_of(by("Boon of the Clear Mind")),
            Some(BuffKind::ManaRegen)
        );
    }

    /// why: reported live -- "weapon proc / Vampiric Embrace -> Vampiric
    /// Embrace". The two level bases disagree for any spell whose classes
    /// carry different requirements, so a spell outranked ITSELF.
    #[test]
    fn a_buff_is_never_an_upgrade_over_itself() {
        let ve = spells()
            .iter()
            .find(|s| s.name == "Vampiric Embrace")
            .expect("Vampiric Embrace");
        let levels: Vec<u32> = ve.classes.iter().filter_map(|c| c.level).collect();
        let (lo, hi) = (
            *levels.iter().min().expect("levels"),
            *levels.iter().max().expect("levels"),
        );
        assert!(lo < hi, "the bug needs classes that disagree: {levels:?}");

        // why: exactly the reported shape -- on you at the min basis, best
        // at the caster-class basis, same spell
        assert!(
            !is_upgrade(Some(("Vampiric Embrace", lo)), Some("Vampiric Embrace"), hi),
            "a spell cannot be an upgrade over itself"
        );
        // why: a real upgrade still reads as one
        assert!(is_upgrade(Some(("Breeze", 5)), Some("Clarity"), 29));
        // why: nothing on you is missing, not an upgrade
        assert!(!is_upgrade(None, Some("Clarity"), 29));
        // why: what is on you already IS the best rank
        assert!(!is_upgrade(Some(("Clarity", 29)), Some("Clarity"), 29));
        // why: the override reaches the upgrade judgement -- Vampiric
        // Embrace (SHD 15, NEC 7) is never "upgraded" to the cleric proc
        // line, and a line without an override still is
        assert!(!is_upgrade(
            Some(("Vampiric Embrace", 7)),
            Some("Blessing of the Squire"),
            16
        ));
        assert!(is_upgrade(
            Some(("Blessing of the Page", 8)),
            Some("Blessing of the Squire"),
            16
        ));
    }

    /// why: the file decides -- the cleric heal-on-hit line and Vampiric
    /// Embrace are two kinds, an instant is no kind, and a spell the
    /// file does not know still gets the prose answer
    #[test]
    fn the_games_spa_outranks_the_wikis_prose() {
        let row2 = |name: &str, formula: &str, base: &str, slots: &str| {
            let mut f = vec![String::new(); 173];
            f[1] = name.to_string();
            f[11] = formula.to_string();
            f[12] = base.to_string();
            f[172] = slots.to_string();
            f.join("^")
        };
        // why: formula 3 with no base reads as level*30 -- a long buff
        let row = |name: &str, formula: &str, slots: &str| row2(name, formula, "0", slots);
        let file = crate::spelltimers::parse_text(
            &[
                row("Vampiric Embrace", "50", "1|85|821|0|100|0$2|10|0|0|100|0"),
                row("Blessing of the Squire", "3", "1|121|2|0|100|0"),
                row("Clarity", "3", "1|10|0|0|100|0$2|15|1|0|109|9"),
                row("Harvest", "0", "1|15|1|0|5|0"),
                // why: formula 1 base 4 -- a 4-tick HP transfer, not a buff
                row2(
                    "Shadow Compact",
                    "1",
                    "4",
                    "1|10|0|0|100|0$2|10|0|0|100|0$3|10|0|0|100|0$4|0|20|0|100|0",
                ),
                // why: formula 7 base 50 -- five minutes, re-cast on a timer, kept
                row2(
                    "Berserker Spirit",
                    "7",
                    "50",
                    "1|4|40|0|100|40$2|55|200|0|100|250$3|6|-20|0|100|20",
                ),
            ]
            .join("\n"),
        );
        let f = Some(&file);
        let by = |n: &str| {
            crate::spelldata::spells()
                .iter()
                .find(|s| s.name == n)
                .unwrap_or_else(|| panic!("{n} in the catalog"))
        };
        assert_eq!(
            kind_of_with(by("Vampiric Embrace"), f),
            Some(BuffKind::Proc)
        );
        assert_eq!(
            kind_of_with(by("Blessing of the Squire"), f),
            Some(BuffKind::HealOnHit)
        );
        assert_eq!(
            kind_of(by("Blessing of the Squire")),
            Some(BuffKind::Proc),
            "premise: the prose alone files it as a proc"
        );
        assert_eq!(kind_of_with(by("Clarity"), f), Some(BuffKind::ManaRegen));
        assert_eq!(
            kind_of_with(by("Harvest"), f),
            None,
            "an instant is never a buff"
        );
        assert_eq!(
            kind_of_with(by("Shadow Compact"), f),
            None,
            "a 4-tick heal is not a buff"
        );
        assert_eq!(
            kind_of_with(by("Berserker Spirit"), f),
            Some(BuffKind::Strength),
            "five minutes is a maintained buff"
        );
        assert_eq!(
            kind_of_with(by("Celerity"), f),
            kind_of(by("Celerity")),
            "unknown to the file: the prose answers"
        );
    }

    /// why: Spencer -- "it should be detecting SHD/etc and be suggesting
    /// innates like vampiric embrace". A self-only buff is not a PARTY
    /// buff (nobody can put it on you) and its proc slot states neither
    /// Increase nor Absorb, so it mapped to no kind either -- it could
    /// never be reported missing however plainly the log showed it absent.
    #[test]
    fn a_self_only_proc_buff_is_a_buff() {
        let ve = spells()
            .iter()
            .find(|s| s.name == "Vampiric Embrace")
            .expect("Vampiric Embrace");
        assert!(!is_party_buff(ve), "nobody else can cast it on you");
        assert!(is_self_buff(ve), "but you can cast it on yourself");
        assert_eq!(kind_of(ve), Some(BuffKind::Proc));
        assert!(
            reachable(
                ve,
                crate::gearplanner::era_ix(crate::gearplanner::CURRENT_ERA)
            ),
            "Classic Era"
        );
        assert!(benefits(BuffKind::Proc, &["Shadow Knight".into()]));
        assert!(
            !benefits(BuffKind::Proc, &["Wizard".into()]),
            "never swings"
        );

        // why: a recourse is the effect half of a debuff you cast on an
        // enemy -- never something you are missing
        let rec = spells()
            .iter()
            .find(|s| s.name == "siphon strength recourse")
            .expect("the pack's one classed recourse");
        assert!(is_self_buff(rec), "it looks like a self buff");
        assert!(is_recourse(rec), "but it is a recourse");

        // why: the noise this must NOT let in -- a gate is not a buff
        for name in ["Gate", "Feign Death", "Illusion: Barbarian"] {
            if let Some(s) = spells().iter().find(|s| s.name == name) {
                assert_eq!(kind_of(s), None, "{name} is not a buff");
            }
        }
    }

    /// why: the reported case -- the Shield of Words kind recommended
    /// "Skin of the Shadow", which is Kunark Era AND Necromancer 55, so
    /// nobody on this server can cast it for either reason
    #[test]
    fn an_unreachable_buff_is_never_recommended() {
        let by_name = |n: &str| spells().iter().find(|s| s.name == n).expect(n);

        let shadow = by_name("Skin of the Shadow");
        assert_eq!(shadow.era.as_deref(), Some("Kunark Era"));
        assert!(
            !reachable(
                shadow,
                crate::gearplanner::era_ix(crate::gearplanner::CURRENT_ERA)
            ),
            "a Kunark spell is out of era"
        );
        assert!(
            shadow
                .classes
                .iter()
                .all(|c| c.level.is_some_and(|l| l > u32::from(LEVEL_CAP))),
            "and every rank of it is above the cap"
        );

        // why: the buff it was offered as an upgrade over stays offered
        let words = by_name("Shield of Words");
        assert!(
            reachable(
                words,
                crate::gearplanner::era_ix(crate::gearplanner::CURRENT_ERA)
            ),
            "Classic Era, and castable at 45"
        );
        assert!(words
            .classes
            .iter()
            .any(|c| c.level.is_some_and(|l| l <= u32::from(LEVEL_CAP))));

        // why: an era the scrape never stated is not proof of anything --
        // 196 such spells sit under the cap and are ordinary Classic buffs
        let unknown = spells()
            .iter()
            .find(|s| s.era.is_none())
            .expect("the pack has era-unknown spells");
        assert!(reachable(
            unknown,
            crate::gearplanner::era_ix(crate::gearplanner::CURRENT_ERA)
        ));
    }
}
