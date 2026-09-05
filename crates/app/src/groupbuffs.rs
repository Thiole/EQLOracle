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
    /// why: "Add Proc" -- Vampiric Embrace and its line. A real beneficial
    /// self-buff that mapped to no kind, so it could never be reported
    /// missing however plainly the log showed it absent.
    Proc,
}

impl BuffKind {
    pub fn label(self) -> &'static str {
        match self {
            BuffKind::Proc => "weapon proc",
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
pub fn kind_of(spell: &Spell) -> Option<BuffKind> {
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
        if e.contains("Mana") {
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
            (BuffKind::Proc, Shape::Melee) => 70,
            (BuffKind::Proc, _) => 10,
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
        | BuffKind::Proc => melees,
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
fn reachable(spell: &Spell) -> bool {
    let live = crate::gearplanner::era_ix(crate::gearplanner::CURRENT_ERA);
    match spell.era.as_deref().and_then(crate::gearplanner::era_ix) {
        Some(ix) => live.is_none_or(|l| ix <= l),
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
    if best_spell.is_some_and(|b| b.eq_ignore_ascii_case(name)) {
        return false;
    }
    level < best_level
}

pub fn group_buffs(ing: &Ingest) -> GroupBuffsDto {
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
            if is_recourse(spell) || !(is_party_buff(spell) || (is_me && is_self_buff(spell))) {
                continue;
            }
            let Some(kind) = kind_of(spell) else { continue };
            // why: `benefits` stops a groupmate offering mana regen to a
            // pure melee. It cannot arise for a spell you cast on
            // YOURSELF -- your own class having it settles the question.
            if !is_me && !benefits(kind, &my_classes) {
                continue;
            }
            if !reachable(spell) {
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
        match kind_of(spell) {
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
            lines.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.line.cmp(&b.1.line)));
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
    let upgrades = rows.iter().filter(|r| r.upgrade).count();
    let good = rows.iter().all(|r| r.active.is_some()) && upgrades == 0;
    extra_active.sort();
    GroupBuffsDto {
        good,
        upgrades,
        my_classes,
        party,
        rows,
        extra_active,
    }
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
        let Some(kind) = kind_of(spell) else { continue };
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
        assert!(reachable(ve), "Classic Era");
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
        assert!(!reachable(shadow), "a Kunark spell is out of era");
        assert!(
            shadow
                .classes
                .iter()
                .all(|c| c.level.is_some_and(|l| l > u32::from(LEVEL_CAP))),
            "and every rank of it is above the cap"
        );

        // why: the buff it was offered as an upgrade over stays offered
        let words = by_name("Shield of Words");
        assert!(reachable(words), "Classic Era, and castable at 45");
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
        assert!(reachable(unknown));
    }
}
