//! why: Group Buff Tracker -- "Good" when every beneficial buff the
//! party's confirmed classes can cast on you, that benefits your own
//! class combo, is currently on you; else the missing ones and who could
//! cast them. Party = the group tracker's current roster (pets excluded);
//! a class is confirmed by a /who row in this chain or a dozen combat
//! votes; a level comes from /who only (unknown level = any rank).

use crate::ingest::Ingest;
use crate::spelldata::{spells, Spell};
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
}

impl BuffKind {
    pub fn label(self) -> &'static str {
        match self {
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

/// why: which kinds YOUR combo gets anything out of -- mana regen needs
/// a mana user, haste/attack/STR/DEX a melee class; the rest help everyone
pub fn benefits(kind: BuffKind, my_classes: &[String]) -> bool {
    let uses_mana = my_classes.iter().any(|c| MANA_USERS.contains(&c.as_str()));
    let melees = my_classes
        .iter()
        .any(|c| !PURE_CASTERS.contains(&c.as_str()));
    match kind {
        BuffKind::ManaRegen => uses_mana,
        BuffKind::Haste | BuffKind::Attack | BuffKind::Strength | BuffKind::Dexterity => melees,
        _ => true,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct BuffRowDto {
    pub kind: BuffKind,
    pub label: &'static str,
    /// why: on you right now -- the spell's name when it is
    pub active: Option<String>,
    /// why: the best spell of this kind someone in the party can cast, and who
    pub best_spell: String,
    pub casters: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PartyMemberDto {
    pub name: String,
    pub classes: Vec<String>,
    pub level: Option<u8>,
    pub confirmed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct GroupBuffsDto {
    /// why: every expected kind covered -- the one word the widget leads with
    pub good: bool,
    pub my_classes: Vec<String>,
    pub party: Vec<PartyMemberDto>,
    pub rows: Vec<BuffRowDto>,
    /// why: buffs on you the party can't account for (someone outside
    /// the group, or an unknown class) -- listed, never counted against
    pub extra_active: Vec<String>,
}

/// why: a class counts as confirmed with this many combat votes, same
/// bar the ally table turns green at
const CONFIRMED_VOTES: u32 = 12;

pub fn group_buffs(ing: &Ingest) -> GroupBuffsDto {
    let now = ing.now_ms();
    let my_classes: Vec<String> = ing
        .store
        .names
        .get("You")
        .map(|y| {
            let by_visit = ing
                .classes
                .configuration_of_visit(y.0, ing.zone.index_at(now));
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
    for (name, _, _, _) in ing.groups.current_members(now) {
        if name.eq_ignore_ascii_case("You") || ing.effective_kind(&name, now) == Kind::Pet {
            continue;
        }
        let (classes, level, confirmed) = match ing.ally_who(&name, now) {
            Some((lvl, trio)) => (trio.to_vec(), Some(lvl), true),
            None => {
                let (c, votes) = ing.ally_classes(&name, now);
                (c, None, votes >= CONFIRMED_VOTES)
            }
        };
        party.push(PartyMemberDto {
            name,
            classes,
            level,
            confirmed,
        });
    }

    // why: per kind, the best (highest-level) party-castable spell and
    // everyone who could cast it -- confirmed classes only
    let mut best: BTreeMap<BuffKind, (u32, String, HashSet<String>)> = BTreeMap::new();
    for member in party.iter().filter(|m| m.confirmed) {
        for spell in spells().iter().filter(|s| is_party_buff(s)) {
            let Some(kind) = kind_of(spell) else { continue };
            if !benefits(kind, &my_classes) {
                continue;
            }
            let castable = spell.classes.iter().find(|sc| {
                member.classes.iter().any(|c| c == &sc.class)
                    && member
                        .level
                        .is_none_or(|lvl| sc.level.is_none_or(|need| need <= u32::from(lvl)))
            });
            let Some(sc) = castable else { continue };
            let rank = sc.level.unwrap_or(0);
            let e = best
                .entry(kind)
                .or_insert_with(|| (0, spell.name.clone(), HashSet::new()));
            if rank > e.0 || e.2.is_empty() {
                if rank > e.0 {
                    e.2.clear();
                }
                e.0 = rank;
                e.1 = spell.name.clone();
            }
            e.2.insert(member.name.clone());
        }
    }

    // why: what's on you, by kind
    let mut active_by_kind: HashMap<BuffKind, String> = HashMap::new();
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
                active_by_kind
                    .entry(k)
                    .or_insert_with(|| spell_name.clone());
            }
            None => extra_active.push(spell_name.clone()),
        }
    }

    let rows: Vec<BuffRowDto> = best
        .into_iter()
        .map(|(kind, (_, best_spell, casters))| {
            let mut casters: Vec<String> = casters.into_iter().collect();
            casters.sort();
            BuffRowDto {
                kind,
                label: kind.label(),
                active: active_by_kind.get(&kind).cloned(),
                best_spell,
                casters,
            }
        })
        .collect();
    let good = rows.iter().all(|r| r.active.is_some());
    extra_active.sort();
    GroupBuffsDto {
        good,
        my_classes,
        party,
        rows,
        extra_active,
    }
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
}
