//! Encounter detection by damage graph, and entity classification.
//!
//! Design notes: `docs/design/encounters.md`

use crate::fold_key;
use std::collections::HashMap;

pub type Millis = i64;

/// why: game-grounded defaults, but every judgement call stays settable
#[derive(Debug, Clone, Copy)]
pub struct Policy {
    /// why: 10s, for RESOLVED fights only (every non-ally entity dead)
    /// -- matches the real out-of-combat recovery boundary, and closing
    /// fast here is what keeps back-to-back pulls from merging into one
    /// mega-encounter (measured: a flat 30s window halved the log's
    /// kill count exactly that way). Unresolved fights use
    /// idle_unresolved_ms instead -- see its own doc.
    pub idle_ms: Millis,

    /// why: 5 minutes -- a SAFETY net, not a rule. A fight with no kill
    /// and no end-of-combat flag does not time out on its own: "it should
    /// be 10 seconds after a kill; if there's actions, even mesmerization,
    /// it extends until a kill or a flag to possibly end combat -- charm,
    /// port, memwipe". Zoning already closes every fight; charm and a
    /// mem blur arm the 10s window (see Live::flagged). This only catches
    /// a fight the log genuinely never resolved (a mob that just walked
    /// off), so it can't sit open forever.
    pub idle_unresolved_ms: Millis,

    /// why: a re-engaged fled mob within this window is one kill, not two
    pub link_ms: Millis,

    /// why: 0 -- was 6s for a same-named survivor, withdrawn: "it should
    /// be 10 total, not 16". Kept as a knob; Live::dupe still records
    /// the survivor for anything that wants to know.
    pub dupe_grace_ms: Millis,

    /// why: off means split-only, useful when a crowded zone adds noise
    pub transitive: bool,

    /// why: caps merging -- a raid is dozens, a runaway chain looks the same
    pub max_entities: Option<usize>,
}

impl Default for Policy {
    fn default() -> Self {
        Policy {
            idle_ms: 10_000,
            idle_unresolved_ms: 300_000,
            link_ms: 60_000,
            dupe_grace_ms: 0,
            transitive: true,
            max_entities: None,
        }
    }
}

impl Policy {
    pub fn idle_secs(mut self, s: f64) -> Self {
        self.idle_ms = (s * 1000.0) as Millis;
        self
    }
    pub fn idle_unresolved_secs(mut self, s: f64) -> Self {
        self.idle_unresolved_ms = (s * 1000.0) as Millis;
        self
    }
    pub fn link_secs(mut self, s: f64) -> Self {
        self.link_ms = (s * 1000.0) as Millis;
        self
    }
    pub fn dupe_grace_secs(mut self, s: f64) -> Self {
        self.dupe_grace_ms = (s * 1000.0) as Millis;
        self
    }
    pub fn no_transitive(mut self) -> Self {
        self.transitive = false;
        self
    }
    pub fn cap_entities(mut self, n: usize) -> Self {
        self.max_entities = Some(n);
        self
    }
}

/// why: certainty descends down the list
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// why: confirmed via a player-only channel
    Player,
    /// why: owner named in "<Owner> pet", damage attributes cleanly
    Pet,
    /// why: most are NPCs, some are unspoken players -- not called Npc
    Unproven,
}

/// why: a pet-suffixed name, either side's -- "X`s warder" (player's),
/// "a dracoliche pet" (mob's own). A pet dying is not a kill: it must
/// neither arm the fast post-kill idle close nor confirm anything
/// (asked directly: "a pet dying isn't a kill though").
pub fn is_pet_suffixed(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    l.ends_with(" pet") || l.ends_with(" warder")
}

/// why: owner name from a possessive "<Owner>'s pet" suffix, else None.
/// "warder" too -- a Beastlord's warder logs as "X`s warder", never
/// "X`s pet", and the miss put real warders on the ENEMY side of the
/// meter ("Michele`s warder", 38k damage into enemies across 5 fights,
/// pet_side_check.rs on a real log 2026-08-31).
fn pet_owner(name: &str) -> Option<&str> {
    let base = name
        .strip_suffix(" pet")
        .or_else(|| name.strip_suffix(" warder"))?;
    let owner = base
        .strip_suffix("'s")
        .or_else(|| base.strip_suffix("`s"))?;
    (!owner.is_empty()).then_some(owner)
}

/// why: classification is monotonic -- evidence promotes, never demotes
#[derive(Debug, Default)]
pub struct Entities {
    kind: HashMap<String, Kind>,
    owner: HashMap<String, String>,
    display: HashMap<String, String>,
}

impl Entities {
    fn note_seen(&mut self, key: &str, name: &str) {
        self.display
            .entry(key.to_string())
            .or_insert_with(|| name.to_string());
    }

    /// why: NPCs never use player-only channels -- reliable player proof
    pub fn note_player_channel(&mut self, name: &str) {
        self.promote_to_player(name);
    }

    fn promote_to_player(&mut self, name: &str) {
        let key = fold_key(name);
        self.note_seen(&key, name);
        self.kind.insert(key, Kind::Player);
    }

    /// why: possessive "X's pet" is a player's; bare "X pet" is a mob's own
    pub fn observe(&mut self, name: &str) -> Kind {
        let key = fold_key(name);
        self.note_seen(&key, name);
        if let Some(&k) = self.kind.get(&key) {
            return k;
        }
        let k = match pet_owner(name) {
            Some(owner) => {
                self.owner.insert(key.clone(), owner.to_string());
                Kind::Pet
            }
            None => Kind::Unproven,
        };
        self.kind.insert(key, k);
        k
    }

    pub fn kind(&self, name: &str) -> Kind {
        self.kind
            .get(&fold_key(name))
            .copied()
            .unwrap_or(Kind::Unproven)
    }

    pub fn owner_of(&self, name: &str) -> Option<&str> {
        self.owner.get(&fold_key(name)).map(|s| s.as_str())
    }

    /// why: first-seen casing, so callers intern to one identity elsewhere too
    pub fn display_name<'a>(&'a self, name: &'a str) -> &'a str {
        self.display
            .get(&fold_key(name))
            .map(|s| s.as_str())
            .unwrap_or(name)
    }

    /// Who this damage should count towards: a pet's owner, else the entity.
    pub fn credit<'a>(&'a self, name: &'a str) -> &'a str {
        self.owner_of(name).unwrap_or(name)
    }

    pub fn players(&self) -> impl Iterator<Item = &str> {
        self.kind
            .iter()
            .filter(|(_, &k)| k == Kind::Player)
            .map(|(k, _)| {
                self.display
                    .get(k)
                    .map(|s| s.as_str())
                    .unwrap_or(k.as_str())
            })
    }
}

/// Component id. Stable for the life of the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EncId(pub u32);

#[derive(Debug, Clone)]
pub struct Live {
    pub id: EncId,
    pub start_ms: Millis,
    pub last_ms: Millis,
    pub entities: Vec<String>,
    /// why: a multi-mob pull outlives one death, still one fight
    pub slain: Vec<String>,
    pub events: u32,
    /// why: flags a merged encounter's per-source split as less trustworthy
    pub merged: bool,
    /// why: a slain name took damage again -- a same-named mob is still
    /// up, so a death here doesn't mean the pull is over (Policy::dupe_grace_ms)
    pub dupe: bool,
    /// why: an end-of-combat signal landed (a charm on this fight's mob,
    /// a mem blur) -- "possibly" over: arms the short window like a kill
    pub flagged: bool,
}

#[derive(Debug, Clone)]
pub struct Closed {
    pub id: EncId,
    pub start_ms: Millis,
    pub end_ms: Millis,
    pub entities: Vec<String>,
    pub slain: Vec<String>,
    pub events: u32,
    pub merged: bool,
    /// why: links a re-engage -- UI shows one kill, DPS windows stay separate
    pub links_to: Option<EncId>,
    /// why: Some(keeper) when this record is a merge CORPSE -- the
    /// encounter was absorbed into `keeper` mid-fight, never really
    /// ended. Consumers must fold it into the keeper, not surface it:
    /// surfacing minted zero-length "reset" fights and, when the corpse
    /// carried a slain copy, a second kill for one death (reported
    /// live: dracoliche "reset again... marks it as a kill though").
    pub absorbed_into: Option<EncId>,
}

impl Closed {
    pub fn duration_ms(&self) -> Millis {
        self.end_ms - self.start_ms
    }
}

/// why: connected-component encounter detection, expires after idle_ms
pub struct Builder {
    pub policy: Policy,
    /// why: linking only through a non-player, or every fight would chain
    pub entities: Entities,
    next: u32,
    of: HashMap<String, EncId>,
    live: HashMap<EncId, Live>,
    pub closed: Vec<Closed>,
    /// why: recently closed unslain targets, for the re-engage link
    recent: HashMap<String, (EncId, Millis)>,
}

impl Default for Builder {
    fn default() -> Self {
        Builder::new(Policy::default())
    }
}

impl Builder {
    pub fn new(policy: Policy) -> Self {
        Builder {
            policy,
            entities: Entities::default(),
            next: 0,
            of: HashMap::new(),
            live: HashMap::new(),
            closed: Vec::new(),
            recent: HashMap::new(),
        }
    }

    /// One damage edge. Returns the encounter it belongs to.
    pub fn damage(&mut self, ts: Millis, actor: &str, target: &str) -> EncId {
        self.expire(ts);
        self.entities.observe(actor);
        self.entities.observe(target);

        let a = self.of.get(&fold_key(actor)).copied();
        let b = self.of.get(&fold_key(target)).copied();

        let id = match (a, b) {
            (None, None) => self.open(ts, actor, target),
            (Some(x), None) => {
                self.attach(x, target, ts);
                x
            }
            (None, Some(y)) => {
                self.attach(y, actor, ts);
                y
            }
            (Some(x), Some(y)) if x == y => x,
            (Some(x), Some(y)) => {
                if self.policy.transitive && self.may_merge(x, y) {
                    self.merge(x, y)
                } else {
                    // why: actor joins, target stays the fight's anchor
                    self.attach(y, actor, ts);
                    y
                }
            }
        };

        if let Some(e) = self.live.get_mut(&id) {
            e.last_ms = ts;
            e.events += 1;
            // why: either side -- a "slain" name swinging again is the
            // same proof of a same-named survivor as one being hit
            if !e.dupe
                && e.slain
                    .iter()
                    .any(|n| fold_key(n) == fold_key(target) || fold_key(n) == fold_key(actor))
            {
                e.dupe = true;
            }
        }
        id
    }

    fn may_merge(&self, x: EncId, y: EncId) -> bool {
        match self.policy.max_entities {
            None => true,
            Some(cap) => {
                let n = self.live.get(&x).map_or(0, |e| e.entities.len())
                    + self.live.get(&y).map_or(0, |e| e.entities.len());
                n <= cap
            }
        }
    }

    fn open(&mut self, ts: Millis, a: &str, b: &str) -> EncId {
        let id = EncId(self.next);
        self.next += 1;
        self.live.insert(
            id,
            Live {
                id,
                start_ms: ts,
                last_ms: ts,
                entities: vec![a.to_string(), b.to_string()],
                slain: Vec::new(),
                events: 0,
                merged: false,
                dupe: false,
                flagged: false,
            },
        );
        self.of.insert(fold_key(a), id);
        self.of.insert(fold_key(b), id);
        id
    }

    fn attach(&mut self, id: EncId, name: &str, _ts: Millis) {
        if let Some(e) = self.live.get_mut(&id) {
            if !e.entities.iter().any(|n| fold_key(n) == fold_key(name)) {
                e.entities.push(name.to_string());
            }
        }
        self.of.insert(fold_key(name), id);
    }

    fn merge(&mut self, x: EncId, y: EncId) -> EncId {
        let (keep, gone) = if x.0 <= y.0 { (x, y) } else { (y, x) };
        if let Some(src) = self.live.remove(&gone) {
            if let Some(dst) = self.live.get_mut(&keep) {
                for n in &src.entities {
                    if !dst.entities.iter().any(|m| fold_key(m) == fold_key(n)) {
                        dst.entities.push(n.clone());
                    }
                }
                dst.slain.extend(src.slain.iter().cloned());
                dst.events += src.events;
                dst.start_ms = dst.start_ms.min(src.start_ms);
                dst.merged = true;
                dst.dupe |= src.dupe;
                dst.flagged |= src.flagged;
            }
            for n in &src.entities {
                self.of.insert(fold_key(n), keep);
            }
            // why: `gone` never reaches close(); its Closed is a merge
            // corpse, marked absorbed_into so consumers reparent it
            self.closed.push(Closed {
                id: src.id,
                start_ms: src.start_ms,
                end_ms: src.last_ms,
                entities: src.entities,
                slain: src.slain,
                events: src.events,
                merged: true,
                links_to: None,
                absorbed_into: Some(keep),
            });
        }
        keep
    }

    /// why: target leaves combat, fight continues if anything else is up
    pub fn death(&mut self, ts: Millis, target: &str) {
        if let Some(id) = self.of.remove(&fold_key(target)) {
            if let Some(e) = self.live.get_mut(&id) {
                e.last_ms = ts;
                if !e.slain.iter().any(|n| fold_key(n) == fold_key(target)) {
                    e.slain.push(target.to_string());
                }
            }
        }
        // why: not cleared from `recent` -- would break the backward link
    }

    /// why: a signal that the fight MAY be over without a kill -- a mob
    /// charmed (it changed sides), "Your enemies have forgotten you!" (a
    /// mem blur landed). Arms the 10s window on that entity's fight; any
    /// further action still extends it, so a fight that goes on goes on.
    pub fn flag_end(&mut self, name: &str, ts: Millis) {
        if let Some(&id) = self.of.get(&fold_key(name)) {
            if let Some(e) = self.live.get_mut(&id) {
                e.flagged = true;
                if ts > e.last_ms {
                    e.last_ms = ts;
                }
            }
        }
    }

    /// why: active CC on a fight's own entity means the fight is paused,
    /// not over -- a mezzed mob writes no damage lines, and the idle
    /// clock alone would close the fight mid-mezz as a bogus "reset"
    /// (7.6% of a real log's resets had the target mezzed within 20s of
    /// the close -- reset_check.rs). Refreshing last_ms buys the fight
    /// another idle window from the CC line itself.
    pub fn touch_entity(&mut self, name: &str, ts: Millis) {
        if let Some(&id) = self.of.get(&fold_key(name)) {
            if let Some(e) = self.live.get_mut(&id) {
                if ts > e.last_ms {
                    e.last_ms = ts;
                }
            }
        }
    }

    /// why: call every event/tick, or a quiet fight never closes.
    /// Two idle windows -- see Policy: a fight where SOMETHING has died
    /// closes on the short one (a concluded pull goes quiet because it's
    /// over, and closing fast is what keeps the next pull from merging
    /// in); a fight with zero kills yet gets the long one -- quiet there
    /// means mezz, a fled mob, or a med break, not "over". NOT keyed on
    /// "every entity slain": an unspoken groupmate reads Unproven (the
    /// documented detection ceiling) and held every fight to the long
    /// window, merging back-to-back pulls -- measured, reset_check.rs.
    pub fn expire(&mut self, now: Millis) {
        let stale: Vec<EncId> = self
            .live
            .iter()
            .filter(|(_, e)| {
                // why: pet deaths don't resolve a fight -- "a pet dying
                // isn't a kill"; only a non-pet death arms the short window
                let real_kill = e.slain.iter().any(|n| !is_pet_suffixed(n));
                let idle = if real_kill || e.flagged {
                    self.policy.idle_ms + if e.dupe { self.policy.dupe_grace_ms } else { 0 }
                } else {
                    self.policy.idle_unresolved_ms
                };
                now - e.last_ms > idle
            })
            .map(|(&id, _)| id)
            .collect();
        for id in stale {
            self.close(id);
        }
        let link = self.policy.link_ms;
        self.recent.retain(|_, (_, t)| now - *t <= link);
    }

    fn close(&mut self, id: EncId) {
        let e = match self.live.remove(&id) {
            Some(e) => e,
            None => return,
        };
        for n in &e.entities {
            if self.of.get(&fold_key(n)) == Some(&id) {
                self.of.remove(&fold_key(n));
            }
        }

        // why: links via a surviving non-player -- players would chain everything
        // lookup: any non-player may carry a link back, dead or not
        // carry: only one that left this fight ALIVE can be re-engaged
        let is_player = |n: &String| self.entities.kind(n) == Kind::Player;

        let mut links_to = None;
        for n in e.entities.iter().filter(|n| !is_player(n)) {
            if let Some(&(prev, _)) = self.recent.get(&fold_key(n)) {
                links_to = Some(prev);
                break;
            }
        }

        let carry: Vec<String> = e
            .entities
            .iter()
            .filter(|n| !is_player(n) && !e.slain.iter().any(|s| fold_key(s) == fold_key(n)))
            .cloned()
            .collect();
        for n in carry {
            self.recent.insert(fold_key(&n), (id, e.last_ms));
        }

        self.closed.push(Closed {
            id: e.id,
            start_ms: e.start_ms,
            end_ms: e.last_ms,
            entities: e.entities,
            slain: e.slain,
            events: e.events,
            merged: e.merged,
            links_to,
            absorbed_into: None,
        });
    }

    pub fn close_all(&mut self, _now: Millis) {
        let ids: Vec<EncId> = self.live.keys().copied().collect();
        for id in ids {
            self.close(id);
        }
    }

    pub fn live_encounters(&self) -> impl Iterator<Item = &Live> {
        self.live.values()
    }

    /// why: direct lookup for a caller that already has the id
    pub fn live(&self, id: EncId) -> Option<&Live> {
        self.live.get(&id)
    }

    pub fn live_count(&self) -> usize {
        self.live.len()
    }

    pub fn encounter_of(&self, entity: &str) -> Option<EncId> {
        self.of.get(&fold_key(entity)).copied()
    }
}
