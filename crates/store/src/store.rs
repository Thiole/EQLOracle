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
    /// `You have looted <item> from <corpse>.` -- `actor` is the looter
    /// (always "You"; the log never reports anyone else's loot), `target`
    /// is the corpse's mob name, `ability` is the item name reusing the
    /// `Abilities` interner the same way `record_miss` reuses it for the
    /// synthetic "Miss" ability -- an item isn't really an ability, but the
    /// table is already exactly "interned name -> per-row metadata", and
    /// `amount` is always `1` (the log gives no quantity).
    Loot,
    /// `You gain experience!` -- `actor`/`target` are both the player
    /// (always self-directed; the log never reports anyone else's XP),
    /// `ability` reuses the interner to carry the gain's scope
    /// ("solo"/"party"/"group"/"raid") the same way `Loot` reuses it for
    /// an item name, and `amount` is the percentage in milli-percent
    /// (`11.000%` -> `11000`) rather than the bare float, since this
    /// column is a `u64` -- divide by `1000.0` to recover the percentage.
    /// See `eqlp_app::ingest::Ingest::record_xp` for exactly how this gets
    /// built and why `enc` is only ever a best-effort guess, filled in
    /// after the fact when a kill's own death line follows, and left as
    /// `NO_ENCOUNTER` for the (real, confirmed against actual log data)
    /// case of quest-turn-in XP, which shares this exact line but has no
    /// kill to attribute to.
    Xp,
    /// Platinum/gold/silver/copper actually received -- `actor`/`target`
    /// are both the player (always self-directed), `ability` reuses the
    /// interner to carry the source ("corpse"/"autosell"/"vendor", one per
    /// real log line shape `Ingest::record_currency`'s callers cover), and
    /// `amount` is the *total in copper* (1 platinum = 1000 copper here --
    /// see `Ingest::parse_currency_copper`'s doc for the full conversion
    /// and why parsing lands on copper as the one common unit), not the
    /// bare denomination the log happened to phrase it in.
    Currency,
}

/// Widened from `u16` to fit the 4 mitigation bits below -- 13 bits were
/// already spoken for (0-12), only 3 spare.
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
    /// A multi-hit melee special, distinct from `RAMPAGE` -- both are
    /// real, separately-named mechanics in the log (310 real `Flurry`
    /// lines alongside `Rampage`/`Wild Rampage`), not variants of the
    /// same one.
    pub const FLURRY: Flags = 1 << 17;

    /// `EventKind::Cast` outcome, from `eqlp_session::cast::Resolver`.
    /// Mutually exclusive -- exactly one is set once a cast resolves, never
    /// zero and never more than one. A `Cast` row with none of these set is
    /// still open when the store was queried (shouldn't be pushed until
    /// `Resolver` resolves it, but the bit layout makes "unresolved" and
    /// "unconfirmed" distinguishable if that ever changes).
    pub const CAST_LANDED: Flags = 1 << 8;
    pub const CAST_RESISTED: Flags = 1 << 9;
    pub const CAST_INTERRUPTED: Flags = 1 << 10;
    pub const CAST_FIZZLED: Flags = 1 << 11;
    pub const CAST_UNCONFIRMED: Flags = 1 << 12;

    /// A swing that dealt zero damage because the target fully avoided it
    /// -- set on a `Miss`-kind row carrying the *same* ability name a
    /// landed swing of that type would (`Punch`, `Slash`, ...), not a
    /// separate synthetic ability. Mutually exclusive, same stance as the
    /// `CAST_*` bits above: a swing resolves exactly one way. `MITIGATED`
    /// is a convenience OR of all four, not its own outcome -- check it
    /// when only "was this fully avoided, whichever way" matters; check
    /// the specific bit when which way matters.
    pub const MISSED: Flags = 1 << 13;
    pub const BLOCKED: Flags = 1 << 14;
    pub const DODGED: Flags = 1 << 15;
    pub const PARRIED: Flags = 1 << 16;
    pub const MITIGATED: Flags = MISSED | BLOCKED | DODGED | PARRIED;

    /// Free-text flags come from the log verbatim, so mapping is by substring
    /// and unknown text simply sets nothing rather than being dropped loudly.
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
    /// Closed by an ally death, not a confirmed target kill. Mutually
    /// exclusive with `slain`: a kill that also cost an ally still counts
    /// as `slain`, not this.
    pub wiped: bool,
    /// Whatever zone was active the instant this fight opened, interned
    /// once here rather than re-derived from `start_ms` on every query
    /// that needs it. `eqlp-app`'s `Ingest::zone` (a `Spans`) is the
    /// source of truth this gets stamped from at open time (see
    /// `Ingest::current_zone`) -- this field is just a cache of "the
    /// answer as of when it mattered", the same role this struct's own
    /// per-row `tier` column already plays for difficulty. `None` for a
    /// fight that opened before the first zone line this session has seen
    /// (the "Unknown" bucket elsewhere in this codebase).
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
    /// Difficulty tier (0-4) of the zone this row was recorded in, parsed
    /// from the zone name by `crate::zone` in `eqlp-app` -- `eqlp-store`
    /// itself knows nothing about EQL's tier naming, this is just an opaque
    /// per-row byte the app layer fills in and later filters on
    /// (`Filter::tier`). Exists so a score baseline can be scoped to "this
    /// target at this difficulty" without a query-time union over every
    /// past zone visit at a matching tier -- see `Ingest::record_history`.
    pub tier: Vec<u8>,

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

    /// Position of `id` in the live vec, or `None` if it has been evicted.
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

    /// Safety net, not the normal close path (that's `close_encounter`,
    /// driven by `Ingest::drain_closed`): force-closes any still-open
    /// encounter whose own last row is more than `idle_ms` before `now`,
    /// for whatever slips past the graph layer's own closing logic (a bug
    /// there, an edge case not yet found) and would otherwise sit open
    /// forever, its reported duration growing every time it's queried.
    /// Closes at that last row's own timestamp, never `now` -- closing "now"
    /// would inflate the duration by however long it sat unswept, which is
    /// exactly the failure this exists to catch.
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

    /// Changes an open encounter's anchor label after the fact. For when
    /// `Ingest::link` opened a fight on its best guess at the time (the
    /// first damage edge it saw) and a later edge in the same fight proves
    /// a better one -- see `link`'s doc comment for why the first edge
    /// alone is sometimes the wrong guess, and why waiting for proof rather
    /// than reopening the encounter is what fixes it without disturbing
    /// `first`/`last`/anything else about the fight.
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
