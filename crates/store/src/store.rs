//! Columnar event store. Single source of truth.
//!
//! Design notes: `docs/design/store.md`

use crate::ability::{Abilities, AbilityId, Interner, Sym, Tags};
use eqlp_source::Millis;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    Damage,
    Heal,
    Miss,
    Cast,
    Death,
}

pub type Flags = u16;

pub mod flag {
    use super::Flags;
    pub const CRITICAL: Flags = 1 << 0;
    pub const RIPOSTE: Flags = 1 << 1;
    pub const RAMPAGE: Flags = 1 << 2;
    pub const STRIKETHROUGH: Flags = 1 << 3;
    pub const CRIPPLING: Flags = 1 << 4;
    pub const FINISHING: Flags = 1 << 5;
    pub const SLAY_UNDEAD: Flags = 1 << 6;
    pub const DOUBLE_BOW: Flags = 1 << 7;

    /// Free-text flags come from the log verbatim, so mapping is by substring
    /// and unknown text simply sets nothing rather than being dropped loudly.
    pub fn parse(s: &str) -> Flags {
        let mut f = 0;
        for (needle, bit) in [
            ("Critical", CRITICAL),
            ("Riposte", RIPOSTE),
            ("Rampage", RAMPAGE),
            ("Strikethrough", STRIKETHROUGH),
            ("Crippling", CRIPPLING),
            ("Finishing", FINISHING),
            ("Slay Undead", SLAY_UNDEAD),
            ("Double Bow", DOUBLE_BOW),
        ] {
            if s.contains(needle) {
                f |= bit;
            }
        }
        f
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EncounterId(pub u32);

/// An encounter is a half-open range over the event log plus its identity. It
/// stores no damage of its own: totals are computed from the range.
#[derive(Debug, Clone)]
pub struct Encounter {
    pub id: EncounterId,
    pub target: Sym,
    pub start_ms: Millis,
    pub end_ms: Option<Millis>,
    pub first: u32,
    pub last: u32,
    pub slain: bool,
}

impl Encounter {
    pub fn is_open(&self) -> bool {
        self.end_ms.is_none()
    }
    pub fn range(&self) -> std::ops::Range<usize> {
        self.first as usize..self.last as usize + 1
    }
    pub fn duration_ms(&self, now: Millis) -> Millis {
        self.end_ms.unwrap_or(now) - self.start_ms
    }
}

/// Struct-of-arrays. Grouping touches only the columns it needs, which is the
/// difference between a full scan being a millisecond and being noticeable.
#[derive(Debug, Default)]
pub struct Store {
    pub ts: Vec<Millis>,
    pub kind: Vec<EventKind>,
    pub actor: Vec<Sym>,
    pub target: Vec<Sym>,
    pub ability: Vec<AbilityId>,
    pub amount: Vec<u64>,
    pub flags: Vec<Flags>,
    pub enc: Vec<u32>,

    pub names: Interner,
    pub abilities: Abilities,
    pub encounters: Vec<Encounter>,
    /// Encounters dropped by eviction. `EncounterId` stays stable across
    /// eviction — the UI and the event stream hold ids, so renumbering them
    /// would silently repoint every reference at the wrong fight.
    evicted: u32,
}

pub const NO_ENCOUNTER: u32 = u32::MAX;

impl Store {
    pub fn len(&self) -> usize {
        self.ts.len()
    }
    pub fn is_empty(&self) -> bool {
        self.ts.is_empty()
    }

    pub fn sym(&mut self, s: &str) -> Sym {
        self.names.intern(s)
    }

    pub fn ability_id(&mut self, name: &str, tags: Tags) -> AbilityId {
        let s = self.names.intern(name);
        self.abilities.get_or_add(s, tags)
    }

    /// Append one event. The only way data enters the store.
    #[allow(clippy::too_many_arguments)]
    pub fn push(
        &mut self,
        ts: Millis,
        kind: EventKind,
        actor: Sym,
        target: Sym,
        ability: AbilityId,
        amount: u64,
        flags: Flags,
        enc: u32,
    ) -> u32 {
        self.ts.push(ts);
        self.kind.push(kind);
        self.actor.push(actor);
        self.target.push(target);
        self.ability.push(ability);
        self.amount.push(amount);
        self.flags.push(flags);
        self.enc.push(enc);
        if kind == EventKind::Damage || kind == EventKind::Heal {
            self.abilities.note_amount(ability, amount);
        }
        (self.ts.len() - 1) as u32
    }

    pub fn open_encounter(&mut self, target: Sym, ts: Millis, idx: u32) -> EncounterId {
        let id = EncounterId(self.evicted + self.encounters.len() as u32);
        self.encounters.push(Encounter {
            id,
            target,
            start_ms: ts,
            end_ms: None,
            first: idx,
            last: idx,
            slain: false,
        });
        id
    }

    /// Position of `id` in the live vec, or `None` if it has been evicted.
    #[inline]
    fn slot(&self, id: EncounterId) -> Option<usize> {
        id.0.checked_sub(self.evicted).map(|i| i as usize).filter(|&i| i < self.encounters.len())
    }

    pub fn extend_encounter(&mut self, id: EncounterId, idx: u32) {
        if let Some(e) = self.slot(id).and_then(|i| self.encounters.get_mut(i)) {
            e.last = idx;
        }
    }

    pub fn close_encounter(&mut self, id: EncounterId, ts: Millis, slain: bool) {
        if let Some(e) = self.slot(id).and_then(|i| self.encounters.get_mut(i)) {
            e.end_ms = Some(ts);
            e.slain = slain;
        }
    }

    pub fn encounter(&self, id: EncounterId) -> Option<&Encounter> {
        self.slot(id).and_then(|i| self.encounters.get(i))
    }

    /// Encounters dropped so far. Ids below `evicted` no longer resolve.
    pub fn evicted(&self) -> u32 {
        self.evicted
    }

    pub fn name(&self, s: Sym) -> &str {
        self.names.name(s)
    }

    pub fn ability_name(&self, a: AbilityId) -> &str {
        self.abilities.get(a).map(|x| self.names.name(x.name)).unwrap_or("")
    }

    /// Approximate heap footprint, for deciding when to evict.
    pub fn bytes(&self) -> usize {
        self.len() * (8 + 1 + 4 + 4 + 4 + 8 + 2 + 4)
            + self.encounters.len() * std::mem::size_of::<Encounter>()
    }

    /// Drop the oldest `n` encounters and everything before them.
    ///
    /// A live tail runs for days. Retention is by encounter rather than by
    /// event count so a fight is never half-evicted, which would silently
    /// corrupt its totals.
    pub fn evict_before_encounter(&mut self, n: usize) {
        if n == 0 || n >= self.encounters.len() {
            return;
        }
        let cut = self.encounters[n].first as usize;
        if cut == 0 {
            return;
        }
        self.ts.drain(..cut);
        self.kind.drain(..cut);
        self.actor.drain(..cut);
        self.target.drain(..cut);
        self.ability.drain(..cut);
        self.amount.drain(..cut);
        self.flags.drain(..cut);
        self.enc.drain(..cut);
        self.encounters.drain(..n);
        self.evicted += n as u32;
        let shift = cut as u32;
        for e in &mut self.encounters {
            e.first -= shift;
            e.last -= shift;
        }
    }
}
