//! Columnar event store. Single source of truth.
//!
//! Design notes: `docs/design/store.md`

use crate::ability::{Abilities, AbilityId, Interner, Sym, Tags};
use eqlp_source::Millis;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
    /// why: "You", your pet, or a current groupmate acted in this fight --
    /// false means someone else's fight (or mob vs mob), parsed for clean
    /// data but never surfaced as the player's own combat. Monotonic:
    /// flips true the moment involvement is proven, never back.
    pub involves_you: bool,
    /// why: this encounter was a mid-fight merge corpse, reparented into
    /// another -- its rows now carry the keeper's id and NOTHING may
    /// surface it as a fight (no list row, no pull, no kill, no reset)
    pub absorbed: bool,
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
    /// why: rows a compaction folded into this one (1 for a raw row) --
    /// every reader that counts rows weighs by it, see `compact_before`
    pub count: Vec<u32>,
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
    /// why: backfill grows the columns by doubling; the slack is a third
    /// of the event table on a real log (152 MB held for 107 MB of
    /// events). Called once at the live seam.
    pub fn shrink_to_fit(&mut self) {
        self.ts.shrink_to_fit();
        self.kind.shrink_to_fit();
        self.actor.shrink_to_fit();
        self.target.shrink_to_fit();
        self.ability.shrink_to_fit();
        self.amount.shrink_to_fit();
        self.flags.shrink_to_fit();
        self.enc.shrink_to_fit();
        self.count.shrink_to_fit();
        self.tier.shrink_to_fit();
        self.encounters.shrink_to_fit();
    }

    /// why: once you have left a zone nothing more arrives for its
    /// fights, so their combat rows (Damage/Heal/Miss/Cast) fold into one
    /// row per (fight, actor, target, ability, kind, flags, tier) with
    /// `amount` summed and `count` the rows folded -- totals, hits,
    /// crits, misses, DPS and every ability breakdown read the same
    /// through the query layer; only per-row detail (min/max hit, the
    /// per-second series) collapses. Loot, XP, currency, craft and death
    /// rows stay raw. Rows of a still-open fight, or at/after `cut_ts`,
    /// stay raw. Encounter ranges are remapped; ids never change.
    /// Returns rows removed.
    pub fn compact_before(&mut self, cut_ts: Millis) -> usize {
        let n = self.len();
        if n == 0 {
            return 0;
        }
        let closed: Vec<bool> = self.encounters.iter().map(|e| !e.is_open()).collect();
        let foldable: Vec<bool> = (0..n)
            .map(|i| {
                self.ts[i] < cut_ts
                    && matches!(
                        self.kind[i],
                        EventKind::Damage | EventKind::Heal | EventKind::Miss | EventKind::Cast
                    )
                    && (self.enc[i] == NO_ENCOUNTER
                        || closed.get(self.enc[i] as usize).copied().unwrap_or(true))
            })
            .collect();
        type Key = (u32, Sym, Sym, AbilityId, EventKind, Flags, u8);
        let mut first_of: std::collections::HashMap<Key, usize> = std::collections::HashMap::new();
        // why: pass 1 -- every foldable row joins the first row of its
        // key (kept, in place, which keeps time order); the rest drop
        let mut keep = vec![true; n];
        for i in 0..n {
            if !foldable[i] {
                continue;
            }
            let key: Key = (
                self.enc[i],
                self.actor[i],
                self.target[i],
                self.ability[i],
                self.kind[i],
                self.flags[i],
                self.tier[i],
            );
            match first_of.get(&key) {
                Some(&j) => {
                    self.amount[j] += self.amount[i];
                    self.count[j] += self.count[i];
                    keep[i] = false;
                }
                None => {
                    first_of.insert(key, i);
                }
            }
        }
        let removed = keep.iter().filter(|k| !**k).count();
        if removed == 0 {
            return 0;
        }
        // why: pass 2 -- old index -> new index for the encounter ranges
        let mut new_index = vec![u32::MAX; n];
        let mut w = 0u32;
        for i in 0..n {
            if keep[i] {
                new_index[i] = w;
                w += 1;
            }
        }
        macro_rules! compact {
            ($col:expr) => {{
                let mut w = 0;
                for i in 0..n {
                    if keep[i] {
                        $col.swap(w, i);
                        w += 1;
                    }
                }
                $col.truncate(w);
            }};
        }
        compact!(self.ts);
        compact!(self.kind);
        compact!(self.actor);
        compact!(self.target);
        compact!(self.ability);
        compact!(self.amount);
        compact!(self.flags);
        compact!(self.enc);
        compact!(self.count);
        compact!(self.tier);
        // why: a range's ends may both have been folded away -- walk to
        // the nearest kept row on each side; a fight with no row left
        // keeps an empty range (first > last is what `range()` yields)
        for e in &mut self.encounters {
            let (f, l) = (e.first as usize, e.last as usize);
            let mut nf = f;
            while nf <= l && nf < n && !keep[nf] {
                nf += 1;
            }
            let mut nl = l;
            while nl > nf && nl < n && !keep[nl] {
                nl -= 1;
            }
            if nf > l || nf >= n || !keep[nf] {
                e.first = w;
                e.last = w.wrapping_sub(1);
                continue;
            }
            e.first = new_index[nf];
            e.last = new_index[nl.min(n - 1)];
        }
        removed
    }

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
        self.count.push(1);
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
            involves_you: false,
            absorbed: false,
        });
        id
    }

    /// why: involvement is discovered per-edge, possibly well after open
    /// (a puller's mob only becomes "your fight" once your side acts)
    pub fn mark_involves_you(&mut self, id: EncounterId) {
        if let Some(e) = self.slot(id).and_then(|i| self.encounters.get_mut(i)) {
            e.involves_you = true;
        }
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

    /// why: reparent a merge corpse into its keeper -- rows re-tagged,
    /// keeper's range/start extended to cover them, corpse flagged
    /// absorbed and closed at its own start (zero length, never listed)
    pub fn absorb_encounter(&mut self, corpse: EncounterId, keeper: EncounterId) {
        let Some(c) = self.slot(corpse).and_then(|i| self.encounters.get(i)) else {
            return;
        };
        let (c_first, c_last, c_start) = (c.first, c.last, c.start_ms);
        for i in c_first as usize..=(c_last as usize).min(self.enc.len().saturating_sub(1)) {
            if self.enc[i] == corpse.0 {
                self.enc[i] = keeper.0;
            }
        }
        if let Some(k) = self.slot(keeper).and_then(|i| self.encounters.get_mut(i)) {
            k.first = k.first.min(c_first);
            k.last = k.last.max(c_last);
            k.start_ms = k.start_ms.min(c_start);
        }
        if let Some(c) = self.slot(corpse).and_then(|i| self.encounters.get_mut(i)) {
            c.absorbed = true;
            c.end_ms = Some(c.start_ms);
            c.slain = false;
            c.wiped = false;
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
        self.len() * (8 + 1 + 4 + 4 + 4 + 8 + 4 + 4 + 4 + 1)
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
        self.count.drain(..cut);
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
