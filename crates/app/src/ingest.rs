//! Parses each line once and routes it into the store and the encounter
//! graph. This is the "parsed db": `Store` holds every event ever
//! classified, and nothing here ever reclassifies a line -- the caller
//! (`tail_worker`) is responsible for only handing `process_line` bytes it
//! has not already processed.
//!
//! Bridges two encounter models that intentionally differ:
//! `eqlp_store::Encounter` is a range into the event log with a single
//! target label; `eqlp_session::graph::Builder` is a connected-component
//! fight over possibly many entities (see `docs/design/encounters.md` for
//! why name-keyed encounters are wrong). Something has to translate between
//! them, and that translation is ingestion glue, not parser logic -- it
//! lives here, not in either crate.

use eqlp_core::event::Match;
use eqlp_core::{field, Engine, Outcome};
use eqlp_session::{Builder, EncId, Kind, Policy, Spans, State, Timeline};
use eqlp_source::{Clock, Millis, VirtualClock};
use eqlp_store::{flag, tag, EncounterId, EventKind, Flags, Store, Sym, Tags, NO_ENCOUNTER};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};

/// Running counters for the currently tailed file. Reset whenever the tail
/// target changes (new file, truncation, replacement).
#[derive(Debug, Clone, Default, Serialize)]
pub struct LineCounts {
    pub total: u64,
    pub matched: u64,
    pub unmatched: u64,
    pub headerless: u64,
    pub blank: u64,
    pub by_kind: BTreeMap<String, u64>,
}

impl LineCounts {
    fn add(&mut self, other: &LineCounts) {
        self.total += other.total;
        self.matched += other.matched;
        self.unmatched += other.unmatched;
        self.headerless += other.headerless;
        self.blank += other.blank;
        for (k, v) in &other.by_kind {
            *self.by_kind.entry(k.clone()).or_insert(0) += v;
        }
    }
}

/// One matched line, trimmed to what the live feed shows.
#[derive(Debug, Clone, Serialize)]
pub struct RecentLine {
    pub kind: String,
    pub rule_id: String,
    pub text: String,
}

/// Feed rows kept between drains -- bounds memory if the UI is slow to
/// drain a burst. The frontend keeps its own smaller rolling window on top.
const MAX_PENDING_RECENT: usize = 500;

/// Everything parsed from one tailed file, and the machinery that turns raw
/// matches into store rows and encounters. One instance per tailed file:
/// switching files means a fresh `Ingest`, since row indices and encounter
/// ids are meaningless across a different file.
///
/// Deliberately does not own the `Engine`/`Matcher` used to classify lines.
/// A `Matcher` borrows its `Engine` for its lifetime, and the caller already
/// keeps one alive across the whole tail; threading that borrow through
/// here would make this struct self-referential for no benefit. The caller
/// classifies, this just routes the result.
pub struct Ingest {
    pub store: Store,
    pub encounters: Builder,
    pub zone: Spans,
    /// Entity states (mez/charm/dead) keyed by the same `Sym` the store
    /// uses -- see `docs/design/timeline.md`. Session-wide rather than one
    /// per encounter: `state_at`/`between` already take an explicit time
    /// range, so scoping to one fight is a query, not a second table.
    pub timeline: Timeline,
    enc_map: HashMap<EncId, EncounterId>,
    /// Every entity seen in each store encounter so far, kept current as a
    /// fight grows (a multi-mob pull adds to it) rather than frozen at
    /// whichever mob was hit first -- `store::Encounter` only carries one
    /// label, but a fight can hold several entities. See `link`.
    pub entities_by_enc: HashMap<EncounterId, Vec<String>>,
    /// How far into `encounters.closed` we've synced to the store.
    /// `Builder` only ever appends to that vec, never drains it.
    closed_seen: usize,
    /// Log-time clock: set from the log's own timestamps while replaying
    /// history, then (once `mark_live` is called) also advanced by real
    /// elapsed time between ticks, so a fight that goes quiet during live
    /// tailing closes in near-real-time rather than only when the next
    /// line happens to arrive.
    log_clock: VirtualClock,
    last_wall_ms: Option<Millis>,
    live: bool,
    pub counts: LineCounts,
    pub recent: Vec<RecentLine>,
}

impl Default for Ingest {
    fn default() -> Self {
        Ingest {
            store: Store::default(),
            encounters: Builder::new(Policy::default()),
            zone: Spans::default(),
            timeline: Timeline::default(),
            enc_map: HashMap::new(),
            entities_by_enc: HashMap::new(),
            closed_seen: 0,
            log_clock: VirtualClock::new(0),
            last_wall_ms: None,
            live: false,
            counts: LineCounts::default(),
            recent: Vec::new(),
        }
    }
}

impl Ingest {

    /// Current position on the log's own clock -- milliseconds, no
    /// timezone, same basis as every `LocalTs` in `eqlp-core`.
    pub fn now_ms(&self) -> Millis {
        self.log_clock.now_ms()
    }

    /// Switches from "replaying history as fast as possible" to "live":
    /// from here on, `tick` also advances the log clock by real elapsed
    /// time between calls, not only by timestamps found in new lines.
    /// Backfill must not do this -- parsing twelve days of history in two
    /// seconds must not appear to close every fight in those two seconds.
    pub fn mark_live(&mut self) {
        self.live = true;
        self.last_wall_ms = None; // next tick sets the baseline, not a jump
    }

    /// Call once per line, in order, with the already-computed classification.
    pub fn route(&mut self, engine: &Engine, line: &[u8], outcome: &Outcome) {
        self.counts.total += 1;
        match outcome {
            Outcome::Matched(m) => {
                self.counts.matched += 1;
                let rule = engine.rule(m.rule);
                *self.counts.by_kind.entry(rule.kind.clone()).or_insert(0) += 1;

                if self.live {
                    self.recent.push(RecentLine {
                        kind: rule.kind.clone(),
                        rule_id: rule.id.clone(),
                        text: String::from_utf8_lossy(m.body.slice(line)).into_owned(),
                    });
                    if self.recent.len() > MAX_PENDING_RECENT {
                        let excess = self.recent.len() - MAX_PENDING_RECENT;
                        self.recent.drain(0..excess);
                    }
                }

                let ts_ms = m.ts.secs() * 1000;
                self.log_clock.set_at_least(ts_ms);
                if let Some(action) = extract_action(engine, rule.id.as_str(), m, line) {
                    self.apply(ts_ms, action);
                }
            }
            Outcome::Unmatched { .. } => self.counts.unmatched += 1,
            Outcome::Headerless { .. } => self.counts.headerless += 1,
            Outcome::Blank => self.counts.blank += 1,
        }
    }

    /// Call once per worker loop tick, live or not. Advances the log clock
    /// during live idle stretches and closes fights that have gone quiet.
    pub fn tick(&mut self, wall_now_ms: Millis) {
        if self.live {
            if let Some(last) = self.last_wall_ms {
                let elapsed = (wall_now_ms - last).max(0);
                self.log_clock.set_at_least(self.log_clock.now_ms() + elapsed);
            }
            self.last_wall_ms = Some(wall_now_ms);
        }
        let now = self.log_clock.now_ms();
        self.encounters.expire(now);
        self.drain_closed();
    }

    /// Executes one already-extracted action against the store/graph/zone/
    /// timeline. Never touches `line`/`Match`/`Engine` -- everything it
    /// needed was pulled out by `extract_action`, which is what lets the
    /// same logic run from a sequential merge after parallel classification
    /// (`backfill_parallel`) as well as inline on the live tail thread.
    fn apply(&mut self, ts: Millis, action: Action) {
        match action {
            Action::Damage { src, dst, ability, tags, amount, flags } => {
                self.record_damage(ts, &src, &dst, &ability, tags, amount, flags);
            }
            Action::Heal { src, dst, ability, amount } => {
                let dst = resolve_reflexive(&dst, &src);
                self.record_heal(ts, &src, &dst, &ability, amount);
            }
            Action::Miss { src, dst } => self.record_miss(ts, &src, &dst),
            Action::Death { victim } => self.record_death(ts, &victim),
            Action::Zone { zone } => self.zone.enter(ts, zone),
            Action::Cast { who, spell } => {
                // A cast line proves the ability isn't a weapon proc; no
                // store row needed, just the ability metadata.
                let id = self.store.ability_id(&spell, tag::SPELL);
                self.store.abilities.note_cast(id);
                let caster = self.sym(&who);
                self.clear_dead_if_acting(ts, caster);
            }
            Action::PlayerProof { who } => self.encounters.entities.note_player_channel(&who),
            Action::Mez { who } => {
                let sym = self.sym(&who);
                self.timeline.observed(ts, sym.0, State::Mezzed);
            }
            Action::Charm { who } => {
                let sym = self.sym(&who);
                self.timeline.observed(ts, sym.0, State::Charmed);
            }
            Action::Recovered { who } => {
                let sym = self.sym(&who);
                self.timeline.observed(ts, sym.0, State::Engaged);
            }
        }
    }

    /// Damage is what defines the encounter graph (`docs/design/encounters.md`:
    /// "each damage line is an edge"), so this is the only event kind that
    /// opens a new fight. Everything else attaches to whatever fight is
    /// already open, if any.
    fn record_damage(&mut self, ts: Millis, src: &str, dst: &str, ability: &str, tags: Tags, amount: u64, flags: Flags) {
        let enc = self.link(ts, src, dst);
        let a = self.sym(src);
        let t = self.sym(dst);
        self.clear_dead_if_acting(ts, a);
        let ab = self.store.ability_id(ability, tags);
        let idx = self.store.push(ts, EventKind::Damage, a, t, ab, amount, flags, enc.0);
        self.store.extend_encounter(enc, idx);
        self.drain_closed();
    }

    fn record_heal(&mut self, ts: Millis, src: &str, dst: &str, ability: &str, amount: u64) {
        let enc = self.current_encounter_of(src).or_else(|| self.current_encounter_of(dst));
        let a = self.sym(src);
        let t = self.sym(dst);
        self.clear_dead_if_acting(ts, a);
        let ab = self.store.ability_id(ability, tag::HEAL);
        let idx = self.store.push(ts, EventKind::Heal, a, t, ab, amount, 0, enc.map(|e| e.0).unwrap_or(NO_ENCOUNTER));
        if let Some(id) = enc {
            self.store.extend_encounter(id, idx);
        }
    }

    fn record_miss(&mut self, ts: Millis, src: &str, dst: &str) {
        let enc = self.current_encounter_of(src).or_else(|| self.current_encounter_of(dst));
        let a = self.sym(src);
        let t = self.sym(dst);
        self.clear_dead_if_acting(ts, a);
        let ab = self.store.ability_id("Miss", tag::MELEE);
        let idx = self.store.push(ts, EventKind::Miss, a, t, ab, 0, 0, enc.map(|e| e.0).unwrap_or(NO_ENCOUNTER));
        if let Some(id) = enc {
            self.store.extend_encounter(id, idx);
        }
    }

    fn record_death(&mut self, ts: Millis, victim: &str) {
        self.encounters.death(ts, victim);
        let sym = self.sym(victim);
        self.timeline.observed(ts, sym.0, State::Dead);
        self.drain_closed();
    }

    /// If `actor` was last known Dead and is now doing something (dealing
    /// damage, healing, swinging, or -- via `Action::Cast` -- casting),
    /// that action is itself proof of life. The log rarely states a clean
    /// "you have been resurrected" or "you respawn" line, especially for a
    /// corpse-run recovery, so recovery is inferred from the next thing the
    /// entity does rather than waited for -- the same principle as `Lost`
    /// in `drain_closed`, applied going the other direction. Without this,
    /// a death recorded once stayed the last known state forever, which is
    /// what made the state panel keep reporting "dead" long after a
    /// respawn.
    fn clear_dead_if_acting(&mut self, ts: Millis, actor: Sym) {
        if matches!(self.timeline.state_at(actor.0, ts), Some((State::Dead, _))) {
            self.timeline.inferred(ts, actor.0, State::Engaged);
        }
    }

    /// Interns `name` after resolving it to whatever casing this identity
    /// was first observed under (`Entities::display_name`), and registers
    /// it with the entity table if this is the first time it's been seen
    /// through this path (a heal or miss can name someone before any
    /// damage line does). Without this, the store's symbol table could
    /// split one entity into two syms over a sentence-position casing
    /// difference the same way the encounter graph used to -- see
    /// `docs/design/session.md`, "Case folding".
    fn sym(&mut self, name: &str) -> Sym {
        self.encounters.entities.observe(name);
        let resolved = self.encounters.entities.display_name(name).to_string();
        self.store.sym(&resolved)
    }

    /// Routes one damage edge through the encounter graph, then resolves it
    /// to a store `EncounterId`, opening one the first time this graph
    /// component is seen.
    fn link(&mut self, ts: Millis, actor: &str, target: &str) -> EncounterId {
        let enc_id = self.encounters.damage(ts, actor, target);
        let store_id = if let Some(&id) = self.enc_map.get(&enc_id) {
            id
        } else {
            // Anchor the store encounter's single display label on whichever
            // side isn't the player -- "an armadillo", not "You".
            // `open_encounter` only needs the row index this event *will*
            // get, which is exactly the store's current length before the
            // push that follows.
            let anchor = if target.eq_ignore_ascii_case("you") { actor } else { target };
            let target_sym = self.sym(anchor);
            let idx_hint = self.store.len() as u32;
            let id = self.store.open_encounter(target_sym, ts, idx_hint);
            self.enc_map.insert(enc_id, id);
            id
        };
        if let Some(live) = self.encounters.live(enc_id) {
            self.entities_by_enc.insert(store_id, live.entities.clone());
        }
        store_id
    }

    fn current_encounter_of(&self, name: &str) -> Option<EncounterId> {
        let enc_id = self.encounters.encounter_of(name)?;
        self.enc_map.get(&enc_id).copied()
    }

    /// Syncs newly-closed graph encounters into the store. `Builder::closed`
    /// only grows, so this drains what's new since the last call.
    fn drain_closed(&mut self) {
        while self.closed_seen < self.encounters.closed.len() {
            // Cloned rather than borrowed: everything below needs &mut self
            // (sym() touches both the entity table and the store), which
            // can't coexist with a borrow into `encounters.closed`.
            let c = self.encounters.closed[self.closed_seen].clone();
            if let Some(&store_id) = self.enc_map.get(&c.id) {
                self.store.close_encounter(store_id, c.end_ms, !c.slain.is_empty());
            }

            // Everything that leaves a closed fight alive and unaccounted
            // for left for a reason the log didn't report -- memory blur,
            // pacify, fleeing. Marked Lost/Inferred rather than left
            // looking Engaged forever. Players are excluded: the player
            // ending a fight is not "lost". See docs/design/timeline.md,
            // "Observed vs inferred".
            for name in &c.entities {
                if c.slain.iter().any(|s| s == name) || self.encounters.entities.kind(name) == Kind::Player {
                    continue;
                }
                let sym = self.sym(name);
                if !matches!(self.timeline.state_at(sym.0, c.end_ms), Some((State::Dead, _))) {
                    self.timeline.inferred(c.end_ms, sym.0, State::Lost);
                }
            }

            self.closed_seen += 1;
        }
    }
}

/// One line's meaning, fully extracted to owned data -- independent of the
/// `Match`/`line` it came from, so it can cross a thread boundary. Produced
/// by `extract_action`, consumed by `Ingest::apply`.
enum Action {
    Damage { src: String, dst: String, ability: String, tags: Tags, amount: u64, flags: Flags },
    /// `dst` may still be a reflexive pronoun ("himself") -- resolved in
    /// `apply`, not here; extraction stays a pure read of what the line
    /// literally says.
    Heal { src: String, dst: String, ability: String, amount: u64 },
    Miss { src: String, dst: String },
    Death { victim: String },
    Zone { zone: String },
    Cast { who: String, spell: String },
    PlayerProof { who: String },
    Mez { who: String },
    Charm { who: String },
    /// Charm wearing off, or the player's own mez ending -- both a return
    /// to `State::Engaged`.
    Recovered { who: String },
}

/// Classifies what one matched line means, without mutating anything. A
/// pure function of the rule pack and the match, which is what lets it run
/// on a worker thread during parallel backfill just as well as inline on
/// the live tail thread -- see `backfill_parallel`.
fn extract_action(engine: &Engine, rule_id: &str, m: &Match, line: &[u8]) -> Option<Action> {
    let str_field = |name: &str| -> Option<String> {
        match field::field(engine, m, line, name) {
            field::Value::Str(s) => Some(String::from_utf8_lossy(s).into_owned()),
            _ => None,
        }
    };
    let u64_field = |name: &str| -> Option<u64> {
        match field::field(engine, m, line, name) {
            field::Value::U64(n) => Some(n),
            _ => None,
        }
    };

    match rule_id {
        "melee.hit" => {
            let (src, dst, amount) = (str_field("source")?, str_field("target")?, u64_field("amount")?);
            let verb = str_field("verb").unwrap_or_default();
            let flags = str_field("flag").map(|s| flag::parse(&s)).unwrap_or(0);
            Some(Action::Damage {
                src,
                dst,
                ability: canonical_melee_ability(&verb).to_string(),
                tags: tag::MELEE,
                amount,
                flags,
            })
        }
        "spell.damage" => {
            let (src, dst, amount, spell) =
                (str_field("source")?, str_field("target")?, u64_field("amount")?, str_field("spell")?);
            Some(Action::Damage { src, dst, ability: spell, tags: tag::SPELL, amount, flags: 0 })
        }
        "dot.damage" => {
            let (src, dst, amount, spell) =
                (str_field("source")?, str_field("target")?, u64_field("amount")?, str_field("spell")?);
            Some(Action::Damage { src, dst, ability: spell, tags: tag::SPELL | tag::DOT, amount, flags: 0 })
        }
        "dot.damage_uncredited" => {
            // No caster named -- the log gives us nothing to link this to.
            // Attributed to a placeholder rather than dropped, so the
            // damage still counts against the target's total.
            let (dst, amount, spell) = (str_field("target")?, u64_field("amount")?, str_field("spell")?);
            Some(Action::Damage {
                src: "unknown".to_string(),
                dst,
                ability: spell,
                tags: tag::SPELL | tag::DOT,
                amount,
                flags: 0,
            })
        }
        "ds.damage" => {
            // `source` names the effect, not the entity: "Tranixx Darkpaw's
            // flames", "YOUR thorns" -- always the shield wearer's name (or
            // "YOUR" for the player) plus a possessive and the shield's
            // flavour word. Split it so the wearer is the actor, like every
            // other damage line, instead of "Tranixx Darkpaw's flames"
            // silently interning as its own entity separate from Tranixx
            // Darkpaw.
            let (raw_src, dst, amount) = (str_field("source")?, str_field("target")?, u64_field("amount")?);
            let (src, flavour) = split_damage_shield_source(&raw_src);
            Some(Action::Damage {
                src,
                dst,
                ability: format!("Damage Shield ({flavour})"),
                tags: tag::DAMAGE_SHIELD | tag::PROC,
                amount,
                flags: 0,
            })
        }
        "heal.by_spell" => {
            let (src, dst, amount, spell) =
                (str_field("source")?, str_field("target")?, u64_field("amount")?, str_field("spell")?);
            Some(Action::Heal { src, dst, ability: spell, amount })
        }
        "heal.plain" => {
            let (src, dst, amount) = (str_field("source")?, str_field("target")?, u64_field("amount")?);
            Some(Action::Heal { src, dst, ability: "Heal".to_string(), amount })
        }
        "melee.miss" => {
            let (src, dst) = (str_field("source")?, str_field("target")?);
            Some(Action::Miss { src, dst })
        }
        "cast.begin" | "sing.begin" => {
            let who = str_field("source")?;
            let spell = str_field("spell").or_else(|| str_field("song"))?;
            Some(Action::Cast { who, spell })
        }
        "death.you_slew" | "death.other" | "death.plain" => Some(Action::Death { victim: str_field("victim")? }),
        "death.you_died" => {
            // Synthesised, not read from the log -- fold_key makes it match
            // whatever casing "you"/"You" was seen under.
            Some(Action::Death { victim: "You".to_string() })
        }
        "zone.enter" => Some(Action::Zone { zone: str_field("zone")? }),
        "state.mesmerized" => Some(Action::Mez { who: str_field("who")? }),
        "state.charmed" => Some(Action::Charm { who: str_field("who")? }),
        "state.charm_broken" | "state.you_mesmerized" => {
            Some(Action::Recovered { who: str_field("who").unwrap_or_else(|| "You".to_string()) })
        }
        "chat.channel" => Some(Action::PlayerProof { who: str_field("who")? }),
        "chat.directed" => {
            // Only the channels that are provably player-to-player.
            // `says`/`shouts`/`auctions` are excluded on purpose -- NPCs
            // use `says` too, so it proves nothing. See
            // docs/design/encounters.md, "Entity classification".
            let (who, chan) = (str_field("who")?, str_field("chan")?);
            let player_only = matches!(
                chan.as_str(),
                "tells you" | "tells the guild" | "tells the group" | "tell your party" | "tell the guild" | "tell the group"
            );
            player_only.then_some(Action::PlayerProof { who })
        }
        _ => None,
    }
}

/// Melee verb -> canonical ability name. The pack's regex alternates both
/// third-person ("slashes") and the base form EQ uses for "You" ("slash");
/// both mean the same ability. Matches `melee.hit`/`melee.miss`'s exact verb
/// set in `packs/eql.toml` -- see README's ingest note: "backstabs ->
/// Backstab".
fn canonical_melee_ability(verb: &str) -> &'static str {
    match verb {
        "slashes" | "slash" => "Slash",
        "bashes" | "bash" => "Bash",
        "kicks" | "kick" => "Kick",
        "hits" | "hit" => "Hit",
        "cleaves" | "cleave" => "Cleave",
        "punches" | "punch" => "Punch",
        "crushes" | "crush" => "Crush",
        "pierces" | "pierce" => "Pierce",
        "strikes" | "strike" => "Strike",
        "bites" | "bite" => "Bite",
        "frenzies" | "frenzy" => "Frenzy",
        "backstabs" | "backstab" => "Backstab",
        "reaves" | "reave" => "Reave",
        "smites" | "smite" => "Smite",
        "shoots" | "shoot" => "Shoot",
        "slices" | "slice" => "Slice",
        "claws" | "claw" => "Claw",
        "smashes" | "smash" => "Smash",
        "mauls" | "maul" => "Maul",
        "gores" | "gore" => "Gore",
        _ => "Melee",
    }
}

/// `heal.by_spell`/`heal.plain` write a reflexive pronoun ("himself",
/// "herself", "itself", "yourself") rather than the caster's name repeated,
/// when someone heals themself. Resolved back to the caster so the store
/// attributes the heal to an actual entity instead of the literal word
/// "himself".
fn resolve_reflexive(target: &str, source: &str) -> String {
    match target {
        "himself" | "herself" | "itself" | "yourself" => source.to_string(),
        other => other.to_string(),
    }
}

/// Splits a `ds.damage` `source` capture ("Tranixx Darkpaw's flames",
/// "Bravesirrobin's thorns", "YOUR thorns") into the shield wearer's name
/// and the shield's flavour word. Falls back to treating the whole string
/// as the wearer with a generic flavour if it doesn't match either shape,
/// rather than panicking on a pack/log variant this hasn't seen.
fn split_damage_shield_source(raw: &str) -> (String, String) {
    if let Some(flavour) = raw.strip_prefix("YOUR ") {
        return ("You".to_string(), flavour.to_string());
    }
    if let Some(pos) = raw.rfind("'s ") {
        let (wearer, rest) = raw.split_at(pos);
        let flavour = &rest[3..]; // skip "'s "
        if !wearer.is_empty() && !flavour.is_empty() {
            return (wearer.to_string(), flavour.to_string());
        }
    }
    (raw.to_string(), "Damage Shield".to_string())
}

// ---------------------------------------------------------------- parallel backfill

/// One chunk's worth of classification, ready to be replayed sequentially.
/// `matched` keeps every matched line's timestamp even when it produced no
/// `Action` (a "noise" rule, say), because the log clock still needs to
/// advance past it in order.
struct ChunkResult {
    counts: LineCounts,
    matched: Vec<(Millis, Option<Action>)>,
}

/// Classification only -- the expensive, embarrassingly-parallel part. No
/// access to `Ingest`; a chunk is classified against nothing but the
/// (immutable, `Send + Sync`) `Engine` and its own lines, which is what
/// makes it safe to run on someone else's thread.
fn classify_chunk(engine: &Engine, lines: &[&[u8]]) -> ChunkResult {
    let mut matcher = engine.matcher();
    let mut counts = LineCounts::default();
    let mut matched = Vec::with_capacity(lines.len());
    for &line in lines {
        counts.total += 1;
        match matcher.classify(line) {
            Outcome::Matched(m) => {
                counts.matched += 1;
                let rule = engine.rule(m.rule);
                *counts.by_kind.entry(rule.kind.clone()).or_insert(0) += 1;
                let ts_ms = m.ts.secs() * 1000;
                let action = extract_action(engine, rule.id.as_str(), &m, line);
                matched.push((ts_ms, action));
            }
            Outcome::Unmatched { .. } => counts.unmatched += 1,
            Outcome::Headerless { .. } => counts.headerless += 1,
            Outcome::Blank => counts.blank += 1,
        }
    }
    ChunkResult { counts, matched }
}

/// Splits `raw` into complete lines, CRLF-tolerant, holding back a trailing
/// line with no terminating `\n` -- the game may still be mid-write of it.
/// Same contract as `eqlp_core::frame::Framer` for a single buffer, just
/// without needing a streaming callback (see `backfill_parallel`).
fn framed_lines(raw: &[u8]) -> Vec<&[u8]> {
    if raw.is_empty() {
        return Vec::new();
    }
    let mut parts: Vec<&[u8]> = raw.split(|&b| b == b'\n').collect();
    // `[T]::split` emits a trailing empty slice after a separator at the
    // very end; without one, the trailing slice is the partial line. Either
    // way it is not a complete line, so it is dropped here, not emitted.
    parts.pop();
    parts.into_iter().map(strip_cr).collect()
}

fn strip_cr(line: &[u8]) -> &[u8] {
    match line.split_last() {
        Some((&b'\r', rest)) => rest,
        _ => line,
    }
}

/// Parses a whole buffer (a file's history, read in one shot) across
/// several threads instead of one line at a time on the tail thread.
///
/// `Engine::matcher` is documented as "one matcher per thread" precisely
/// for this: the engine itself is immutable and `Send + Sync`, so
/// classification -- the expensive, regex-bound part -- parallelises
/// cleanly. What can't parallelise is applying the results: the encounter
/// graph and zone spans are order-dependent state machines, not a
/// reduction, so that stays a single sequential pass over the classified
/// output, in original line order. On an N-core machine this trades an
/// O(lines) sequential regex pass for an O(lines) sequential hashmap pass
/// plus an O(lines / N) parallel regex pass -- a real win when
/// classification (measured in `docs/design/parsing.md` at ~900ns/line
/// with captures) dominates the per-line cost, which it does here.
///
/// Framing (splitting `raw` into lines) stays single-threaded and runs
/// first: it's a cheap linear scan, not the bottleneck, and doing it once
/// up front is what lets a trailing partial line (the game mid-write when
/// this read happened) get held back exactly as the live path would,
/// rather than misparsed as a truncated line. Uses `framed_lines` rather
/// than `eqlp_core::frame::Framer` here: `Framer::push`'s callback type is
/// generic over any lifetime (it's built for streaming consumption, one
/// line at a time), so it can't hand back slices borrowed from `raw` for
/// later use the way this needs.
pub fn backfill_parallel(ing: &mut Ingest, engine: &Engine, raw: &[u8], threads: usize) {
    let lines = framed_lines(raw);
    if lines.is_empty() {
        return;
    }

    let threads = threads.max(1).min(lines.len());
    let chunk_size = lines.len().div_ceil(threads);

    let results: Vec<ChunkResult> = std::thread::scope(|scope| {
        lines
            .chunks(chunk_size)
            .map(|chunk| scope.spawn(move || classify_chunk(engine, chunk)))
            .collect::<Vec<_>>()
            .into_iter()
            .map(|handle| handle.join().expect("classify worker panicked"))
            .collect()
    });

    // Sequential merge, in file order (chunks were split contiguously, so
    // iterating results in order is iterating the file in order): this is
    // the part that can't parallelise, but it's hashmap/vec work with no
    // regex left in it.
    for r in results {
        ing.counts.add(&r.counts);
        for (ts_ms, action) in r.matched {
            ing.log_clock.set_at_least(ts_ms);
            if let Some(action) = action {
                ing.apply(ts_ms, action);
            }
        }
    }
}
