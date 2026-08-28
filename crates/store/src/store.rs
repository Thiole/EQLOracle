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
    /// why: item name reuses the Abilities interner, amount always 1
    Loot,
    /// why: amount is milli-percent (u64), enc best-effort or NO_ENCOUNTER
    Xp,
    /// why: amount is total copper, actor/target always the player
    Currency,
    /// why: a tradeskill combine attempt -- ability is the output/
    /// attempted item name (reuses the Abilities interner like Loot),
    /// amount always 1, actor/target always the player. Outcome lives in
    /// flags (flag::CRAFT_SUCCESS / not set = failure), not a separate
    /// bool column -- mirrors how Loot's own auto-sold state is a flag.
    Craft,
}

/// why: widened from u16, only 3 spare bits remained
pub type Flags = u32;

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
    /// why: real, separate mechanic from RAMPAGE -- 310 real log lines
    pub const FLURRY: Flags = 1 << 17;

    /// why: Loot-only -- auto-sold, so not actually still held
    pub const LOOT_AUTO_SOLD: Flags = 1 << 18;

    /// why: Craft-only -- set means the combine succeeded, unset means
    /// "You lacked the skills to fashion..." (a real failure, not unknown)
    pub const CRAFT_SUCCESS: Flags = 1 << 19;
    /// why: Craft-only -- this same combine also hit "You can no longer
    /// advance your skill from making this item." (a separate log line,
    /// correlated onto the row it's about -- see Ingest::record_craft)
    pub const CRAFT_SKILL_CAPPED: Flags = 1 << 20;

    /// why: Cast outcome, mutually exclusive, exactly one set once resolved
    pub const CAST_LANDED: Flags = 1 << 8;
    pub const CAST_RESISTED: Flags = 1 << 9;
    pub const CAST_INTERRUPTED: Flags = 1 << 10;
    pub const CAST_FIZZLED: Flags = 1 << 11;
    pub const CAST_UNCONFIRMED: Flags = 1 << 12;

    /// why: fully-avoided swing, same ability name as a landed one
    pub const MISSED: Flags = 1 << 13;
    pub const BLOCKED: Flags = 1 << 14;
    pub const DODGED: Flags = 1 << 15;
    pub const PARRIED: Flags = 1 << 16;
    pub const MITIGATED: Flags = MISSED | BLOCKED | DODGED | PARRIED;

    /// why: substring match on free-text, unknown text just sets nothing
    pub fn parse(s: &str) -> Flags {
        let mut f = 0;
        for (needle, bit) in [
            ("Critical", CRITICAL),
            ("Riposte", RIPOSTE),
            ("Rampage", RAMPAGE),
            ("Flurry", FLURRY),
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

/// why: half-open range + identity, no damage stored -- totals from the range
#[derive(Debug, Clone)]
pub struct Encounter {
    pub id: EncounterId,
    pub target: Sym,
    pub start_ms: Millis,
    pub end_ms: Option<Millis>,
    pub first: u32,
    pub last: u32,
    pub slain: bool,
    /// why: closed by an ally death, mutually exclusive with `slain`
    pub wiped: bool,
    /// why: zone active at open, cached rather than re-derived per query
    pub zone: Option<Sym>,
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

/// why: struct-of-arrays -- grouping touches only the columns it needs
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
    /// why: opaque per-row difficulty byte, app layer fills/filters it
    pub tier: Vec<u8>,

    pub names: Interner,
    pub abilities: Abilities,
    pub encounters: Vec<Encounter>,
    /// why: EncounterId stays stable across eviction, ids never renumber
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
        tier: u8,
    ) -> u32 {
        self.ts.push(ts);
        self.kind.push(kind);
        self.actor.push(actor);
        self.target.push(target);
        self.ability.push(ability);
        self.amount.push(amount);
        self.flags.push(flags);
        self.enc.push(enc);
        self.tier.push(tier);
        if kind == EventKind::Damage || kind == EventKind::Heal {
            self.abilities.note_amount(ability, amount);
        }
        (self.ts.len() - 1) as u32
    }

    pub fn open_encounter(
        &mut self,
        target: Sym,
        ts: Millis,
        idx: u32,
        zone: Option<Sym>,
    ) -> EncounterId {
        let id = EncounterId(self.evicted + self.encounters.len() as u32);
        self.encounters.push(Encounter {
            id,
            target,
            start_ms: ts,
            end_ms: None,
            first: idx,
            last: idx,
            slain: false,
            wiped: false,
            zone,
        });
        id
    }

    /// why: position in the live vec, None if evicted
    #[inline]
    fn slot(&self, id: EncounterId) -> Option<usize> {
        id.0.checked_sub(self.evicted)
            .map(|i| i as usize)
            .filter(|&i| i < self.encounters.len())
    }

    pub fn extend_encounter(&mut self, id: EncounterId, idx: u32) {
        if let Some(e) = self.slot(id).and_then(|i| self.encounters.get_mut(i)) {
            e.last = idx;
        }
    }

    pub fn close_encounter(&mut self, id: EncounterId, ts: Millis, slain: bool, wiped: bool) {
        if let Some(e) = self.slot(id).and_then(|i| self.encounters.get_mut(i)) {
            e.end_ms = Some(ts);
            e.slain = slain;
            e.wiped = wiped;
        }
    }

    /// why: safety net for stale opens -- closes at last row's own ts
    pub fn close_stale_encounters(&mut self, now: Millis, idle_ms: Millis) {
        for e in &mut self.encounters {
            if e.end_ms.is_some() {
                continue;
            }
            let last_ts = self.ts.get(e.last as usize).copied().unwrap_or(e.start_ms);
            if now - last_ts > idle_ms {
                e.end_ms = Some(last_ts);
            }
        }
    }

    /// why: fixes a fight's anchor when a later edge proves a better one
    pub fn retarget_encounter(&mut self, id: EncounterId, target: Sym) {
        if let Some(e) = self.slot(id).and_then(|i| self.encounters.get_mut(i)) {
            e.target = target;
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
        self.abilities
            .get(a)
            .map(|x| self.names.name(x.name))
            .unwrap_or("")
    }

    /// Approximate heap footprint, for deciding when to evict.
    pub fn bytes(&self) -> usize {
        self.len() * (8 + 1 + 4 + 4 + 4 + 8 + 2 + 4 + 1)
            + self.encounters.len() * std::mem::size_of::<Encounter>()
    }

    /// why: by encounter not event count, so a fight is never half-evicted
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
        self.tier.drain(..cut);
        self.encounters.drain(..n);
        self.evicted += n as u32;
        let shift = cut as u32;
        for e in &mut self.encounters {
            e.first -= shift;
            e.last -= shift;
        }
    }
}
