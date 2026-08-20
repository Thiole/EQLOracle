//! Encounter detection by damage graph, and entity classification.
//!
//! Design notes: `docs/design/encounters.md`

use crate::fold_key;
use std::collections::HashMap;

pub type Millis = i64;

/// Tunables. Defaults are grounded in game behaviour, not fitted to one log —
/// but every one of them is a judgement call, so every one is settable.
#[derive(Debug, Clone, Copy)]
pub struct Policy {
    /// Silence after which an entity leaves combat. Default 10s: the point at
    /// which out-of-combat recovery begins, so it tracks a real game-state
    /// boundary rather than a number tuned to make encounter counts look tidy.
    pub idle_ms: Millis,

    /// Two encounters on a target that never died, resuming within this window,
    /// are treated as one interrupted fight. Keeps `idle_ms` honest without
    /// forcing it wider: a mob that fled and was re-engaged is one kill, but
    /// the DPS windows either side stay separate.
    pub link_ms: Millis,

    /// Merge components when a shared entity appears in both. Off means a fight
    /// is only ever split, never joined — useful when a zone is crowded enough
    /// that transitive links are more noise than signal.
    pub transitive: bool,

    /// Above this many entities, stop merging. A raid legitimately reaches
    /// dozens; a runaway chain in a crowded zone does too. `None` disables the
    /// guard.
    pub max_entities: Option<usize>,
}

impl Default for Policy {
    fn default() -> Self {
        Policy {
            idle_ms: 10_000,
            link_ms: 60_000,
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
    pub fn link_secs(mut self, s: f64) -> Self {
        self.link_ms = (s * 1000.0) as Millis;
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

/// What an entity is. Certainty descends down the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Confirmed human: used a player-only channel.
    Player,
    /// `<Owner> pet` — the log names the owner, so damage attributes cleanly.
    Pet,
    /// Not yet proven to be a player. Most are NPCs; some are players who have
    /// not spoken. Deliberately not called `Npc`.
    Unproven,
}

/// A player-pet owner's name, if `name` is shaped like `` <Owner>'s pet ``
/// (possessive apostrophe or this log's backtick-as-apostrophe stand-in,
/// e.g. `` Di`Zok ``). `None` for a bare `` <name> pet `` with no
/// possessive -- see `Entities::observe`'s doc comment for why that's a
/// mob's own pet, not a player's.
fn pet_owner(name: &str) -> Option<&str> {
    let base = name.strip_suffix(" pet")?;
    let owner = base
        .strip_suffix("'s")
        .or_else(|| base.strip_suffix("`s"))?;
    (!owner.is_empty()).then_some(owner)
}

/// Entity registry. Classification is monotonic: evidence promotes, nothing
/// demotes, so a player who speaks once stays a player.
///
/// Identity maps are keyed by `fold_key`, not the raw name -- see
/// `fold_key` for why. `display` keeps the first-cased spelling seen per
/// key, so lookups are case-insensitive but anything handed back out (e.g.
/// `players()`) still reads the way the log actually wrote it.
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

    /// Called when a name uses a player-only channel (group/guild/raid/General).
    /// NPCs use `says`, never these, so this is one reliable player proof.
    pub fn note_player_channel(&mut self, name: &str) {
        self.promote_to_player(name);
    }

    /// Called when a name deals damage to the same target "You" also damage
    /// within the same fight. The log gives no explicit party-roster line,
    /// but landing damage on the very same mob in the very same fight is,
    /// for all practical purposes, proof of being partied together --
    /// stronger and far more common evidence than chat, which many real
    /// players never use. See `Ingest::note_shared_target` (crate `eqlp-app`)
    /// for how this gets applied, including retroactively to anyone who hit
    /// the mob before "You" landed the hit that confirmed it, and for the
    /// currently-charmed guard that keeps this from permanently promoting a
    /// mob that's only temporarily fighting on your side.
    pub fn note_shared_target(&mut self, name: &str) {
        self.promote_to_player(name);
    }

    fn promote_to_player(&mut self, name: &str) {
        let key = fold_key(name);
        self.note_seen(&key, name);
        self.kind.insert(key, Kind::Player);
    }

    /// Classify on first sight. A *player's* pet is detected by a
    /// possessive ` X's pet` / `` X`s pet `` suffix, which carries the
    /// owner's name — the only ownership marker the log provides. Charmed
    /// mobs never get it and stay `Unproven`.
    ///
    /// A bare `` <name> pet `` with no possessive is a *mob's own*
    /// summoned pet -- `a gnoll pet`, `Priest Amiaz pet`, a raid boss's own
    /// add (`Terror pet`, `Fright pet`) -- not a player's ally. Confirmed
    /// against the reference log: 208 distinct bare `<name> pet`
    /// combatants, every one of them a mob or NPC name, never a proven
    /// player; the one possessive `<name>'s pet` seen was the log owner's
    /// own pet. An earlier version treated any ` pet` suffix as proof of a
    /// player's pet, which put every enemy-summoned pet on the ally side of
    /// `Allegiance::of` -- undercounting incoming damage in the Combat
    /// module and leaking straight into the Monsters module's mob list the
    /// same way an unproven player could.
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

    /// The casing this identity was first observed under, regardless of how
    /// `name` happens to be cased. Callers that intern a name elsewhere
    /// (e.g. a store's own symbol table) should resolve through this first,
    /// so "You" and "you" -- or "an armadillo" and "An armadillo" -- always
    /// intern to the same identity there too, not just here.
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
    /// Targets confirmed dead. An encounter can outlive one death: a multi-mob
    /// pull is one fight.
    pub slain: Vec<String>,
    pub events: u32,
    /// Absorbed another component. Flags aggregates for the UI, since a merged
    /// encounter's per-source split is less trustworthy.
    pub merged: bool,
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
    /// Set when this continues an earlier encounter on a target that never
    /// died. The UI can present linked encounters as one kill while keeping
    /// their DPS windows separate.
    pub links_to: Option<EncId>,
}

impl Closed {
    pub fn duration_ms(&self) -> Millis {
        self.end_ms - self.start_ms
    }
}

/// Builds encounters from damage edges by connected component.
///
/// An edge `(actor, target)` joins both into one component. Components expire
/// after `idle_ms` of silence.
pub struct Builder {
    pub policy: Policy,
    /// Entity classification. Owned here because linking and credit both need
    /// it: a fight may only be linked through a non-player, since the player is
    /// present in every encounter and would chain them all together.
    pub entities: Entities,
    next: u32,
    of: HashMap<String, EncId>,
    live: HashMap<EncId, Live>,
    pub closed: Vec<Closed>,
    /// Recently closed, unslain targets -> the encounter they belonged to.
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
                    // Not merging: the actor joins the target's fight, which
                    // keeps the target's identity as the anchor of the fight.
                    self.attach(y, actor, ts);
                    y
                }
            }
        };

        if let Some(e) = self.live.get_mut(&id) {
            e.last_ms = ts;
            e.events += 1;
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
            }
            for n in &src.entities {
                self.of.insert(fold_key(n), keep);
            }
            // why: `gone` never reaches close() -- push its own Closed record
            // here so its store-side twin still gets an end_ms, instead of
            // sitting open forever with a duration that grows every query.
            self.closed.push(Closed {
                id: src.id,
                start_ms: src.start_ms,
                end_ms: src.last_ms,
                entities: src.entities,
                slain: src.slain,
                events: src.events,
                merged: true,
                links_to: None,
            });
        }
        keep
    }

    /// A death line. The target leaves combat immediately; the encounter
    /// continues if anything else in it is still fighting.
    pub fn death(&mut self, ts: Millis, target: &str) {
        if let Some(id) = self.of.remove(&fold_key(target)) {
            if let Some(e) = self.live.get_mut(&id) {
                e.last_ms = ts;
                if !e.slain.iter().any(|n| fold_key(n) == fold_key(target)) {
                    e.slain.push(target.to_string());
                }
            }
        }
        // Deliberately not cleared from `recent`: a target slain in *this*
        // encounter is already excluded from carrying a link forward by the
        // `slain` filter in `close`, and clearing here would destroy the
        // backward link an interrupted fight depends on.
    }

    /// Close components idle longer than `idle_ms`. Call on every event and on
    /// the UI tick, or a fight that ends quietly never closes.
    pub fn expire(&mut self, now: Millis) {
        let stale: Vec<EncId> = self
            .live
            .iter()
            .filter(|(_, e)| now - e.last_ms > self.policy.idle_ms)
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

        // Link to an earlier encounter through a mob that survived: the same
        // mob, re-engaged, is one kill.
        //
        // Players are excluded. You are in every encounter you fight, so
        // linking through a player would chain an entire evening of unrelated
        // fights into one. Only a non-player that left combat alive can carry
        // the link.
        // Two different conditions, and conflating them is a bug worth naming:
        //
        //   lookup  -- any non-player in this fight may carry a link BACK to an
        //              earlier one. Whether it died here is irrelevant; a mob
        //              that fled and was finally killed is the case the link
        //              exists for.
        //   carry   -- only a non-player that left this fight ALIVE can be
        //              re-engaged later. A corpse cannot.
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

    /// One live encounter by id, for a caller that already has it (e.g.
    /// from the return value of `damage`) rather than needing to scan.
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
