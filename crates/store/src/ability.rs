//! Interned names and ability identity.
//!
//! Design notes: `docs/design/store.md`

use std::collections::HashMap;

/// why: names repeat constantly, so grouping stays integer work
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Sym(pub u32);

#[derive(Debug, Default)]
pub struct Interner {
    map: HashMap<Box<str>, Sym>,
    names: Vec<Box<str>>,
}

impl Interner {
    pub fn intern(&mut self, s: &str) -> Sym {
        if let Some(&x) = self.map.get(s) {
            return x;
        }
        let b: Box<str> = s.into();
        let id = Sym(self.names.len() as u32);
        self.names.push(b.clone());
        self.map.insert(b, id);
        id
    }

    pub fn get(&self, s: &str) -> Option<Sym> {
        self.map.get(s).copied()
    }

    pub fn name(&self, s: Sym) -> &str {
        self.names.get(s.0 as usize).map(|b| &**b).unwrap_or("")
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

/// why: a facet not an identity -- rolls up/filters by mechanism
pub type Tags = u32;

pub mod tag {
    use super::Tags;
    pub const MELEE: Tags = 1 << 0;
    pub const SPELL: Tags = 1 << 1;
    pub const DOT: Tags = 1 << 2;
    /// Fires without a cast line: weapon procs, spellblade.
    pub const PROC: Tags = 1 << 3;
    pub const DAMAGE_SHIELD: Tags = 1 << 4;
    pub const HEAL: Tags = 1 << 5;
    /// Landed damage was reduced below the ability's observed ceiling.
    pub const PARTIAL_RESIST: Tags = 1 << 6;
    /// Attributed to a pet or charmed mob rather than the player directly.
    pub const PET: Tags = 1 << 7;

    pub const ALL: &[(Tags, &str)] = &[
        (MELEE, "melee"),
        (SPELL, "spell"),
        (DOT, "dot"),
        (PROC, "proc"),
        (DAMAGE_SHIELD, "damage-shield"),
        (HEAL, "heal"),
        (PARTIAL_RESIST, "partial-resist"),
        (PET, "pet"),
    ];

    pub fn names(t: Tags) -> Vec<&'static str> {
        ALL.iter()
            .filter(|(b, _)| t & b != 0)
            .map(|(_, n)| *n)
            .collect()
    }
}

/// why: tags accumulate as evidence arrives, PROC corrected on a cast
#[derive(Debug, Clone)]
pub struct Ability {
    pub name: Sym,
    pub tags: Tags,
    /// Highest amount observed. Used to distinguish full hits from partials.
    pub ceiling: u64,
    pub seen_cast: bool,
}

#[derive(Debug, Default)]
pub struct Abilities {
    rows: Vec<Ability>,
    by_sym: HashMap<Sym, u32>,
}

/// Index into the ability table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AbilityId(pub u32);

impl Abilities {
    pub fn get_or_add(&mut self, name: Sym, tags: Tags) -> AbilityId {
        if let Some(&i) = self.by_sym.get(&name) {
            self.rows[i as usize].tags |= tags;
            return AbilityId(i);
        }
        let i = self.rows.len() as u32;
        self.rows.push(Ability {
            name,
            tags,
            ceiling: 0,
            seen_cast: false,
        });
        self.by_sym.insert(name, i);
        AbilityId(i)
    }

    pub fn note_cast(&mut self, id: AbilityId) {
        if let Some(r) = self.rows.get_mut(id.0 as usize) {
            r.seen_cast = true;
            // A cast line proves it is not a weapon proc.
            r.tags &= !tag::PROC;
        }
    }

    pub fn note_amount(&mut self, id: AbilityId, amount: u64) {
        if let Some(r) = self.rows.get_mut(id.0 as usize) {
            if amount > r.ceiling {
                r.ceiling = amount;
            }
        }
    }

    pub fn get(&self, id: AbilityId) -> Option<&Ability> {
        self.rows.get(id.0 as usize)
    }

    pub fn tags(&self, id: AbilityId) -> Tags {
        self.get(id).map_or(0, |a| a.tags)
    }

    pub fn ceiling(&self, id: AbilityId) -> u64 {
        self.get(id).map_or(0, |a| a.ceiling)
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
    pub fn iter(&self) -> impl Iterator<Item = (AbilityId, &Ability)> {
        self.rows
            .iter()
            .enumerate()
            .map(|(i, a)| (AbilityId(i as u32), a))
    }
}
