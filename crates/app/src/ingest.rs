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
use eqlp_session::{Builder, EncId, Policy, Spans};
use eqlp_source::{Clock, Millis, VirtualClock};
use eqlp_store::{flag, tag, EncounterId, EventKind, Flags, Store, Tags, NO_ENCOUNTER};
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
    enc_map: HashMap<EncId, EncounterId>,
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
            enc_map: HashMap::new(),
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
                self.dispatch(engine, rule.id.as_str(), m, line, ts_ms);
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

    fn dispatch(&mut self, engine: &Engine, rule_id: &str, m: &Match, line: &[u8], ts: Millis) {
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
                if let (Some(src), Some(dst), Some(amt)) =
                    (str_field("source"), str_field("target"), u64_field("amount"))
                {
                    let verb = str_field("verb").unwrap_or_default();
                    let flags = str_field("flag").map(|s| flag::parse(&s)).unwrap_or(0);
                    self.record_damage(ts, &src, &dst, canonical_melee_ability(&verb), tag::MELEE, amt, flags);
                }
            }
            "spell.damage" => {
                if let (Some(src), Some(dst), Some(amt), Some(spell)) =
                    (str_field("source"), str_field("target"), u64_field("amount"), str_field("spell"))
                {
                    self.record_damage(ts, &src, &dst, &spell, tag::SPELL, amt, 0);
                }
            }
            "dot.damage" => {
                if let (Some(src), Some(dst), Some(amt), Some(spell)) =
                    (str_field("source"), str_field("target"), u64_field("amount"), str_field("spell"))
                {
                    self.record_damage(ts, &src, &dst, &spell, tag::SPELL | tag::DOT, amt, 0);
                }
            }
            "dot.damage_uncredited" => {
                // No caster named -- the log gives us nothing to link this
                // to. Attributed to a placeholder rather than dropped, so
                // the damage still counts against the target's total.
                if let (Some(dst), Some(amt), Some(spell)) =
                    (str_field("target"), u64_field("amount"), str_field("spell"))
                {
                    self.record_damage(ts, "unknown", &dst, &spell, tag::SPELL | tag::DOT, amt, 0);
                }
            }
            "ds.damage" => {
                if let (Some(src), Some(dst), Some(amt)) =
                    (str_field("source"), str_field("target"), u64_field("amount"))
                {
                    self.record_damage(ts, &src, &dst, "Damage Shield", tag::DAMAGE_SHIELD | tag::PROC, amt, 0);
                }
            }
            "heal.by_spell" => {
                if let (Some(src), Some(dst), Some(amt), Some(spell)) =
                    (str_field("source"), str_field("target"), u64_field("amount"), str_field("spell"))
                {
                    let dst = resolve_reflexive(&dst, &src);
                    self.record_heal(ts, &src, &dst, &spell, amt);
                }
            }
            "heal.plain" => {
                if let (Some(src), Some(dst), Some(amt)) =
                    (str_field("source"), str_field("target"), u64_field("amount"))
                {
                    let dst = resolve_reflexive(&dst, &src);
                    self.record_heal(ts, &src, &dst, "Heal", amt);
                }
            }
            "melee.miss" => {
                if let (Some(src), Some(dst)) = (str_field("source"), str_field("target")) {
                    self.record_miss(ts, &src, &dst);
                }
            }
            "cast.begin" | "sing.begin" => {
                if let Some(spell) = str_field("spell").or_else(|| str_field("song")) {
                    // A cast line proves the ability isn't a weapon proc;
                    // no store row needed, just the ability metadata.
                    let id = self.store.ability_id(&spell, tag::SPELL);
                    self.store.abilities.note_cast(id);
                }
            }
            "death.you_slew" => {
                if let Some(victim) = str_field("victim") {
                    self.record_death(ts, &victim);
                }
            }
            "death.other" | "death.plain" => {
                if let Some(victim) = str_field("victim") {
                    self.record_death(ts, &victim);
                }
            }
            "death.you_died" => {
                // Synthesised, not read from the log -- fold_key makes it
                // match whatever casing "you"/"You" was seen under.
                self.record_death(ts, "You");
            }
            "zone.enter" => {
                if let Some(zone) = str_field("zone") {
                    self.zone.enter(ts, zone);
                }
            }
            "chat.channel" => {
                if let Some(who) = str_field("who") {
                    self.encounters.entities.note_player_channel(&who);
                }
            }
            "chat.directed" => {
                // Only the channels that are provably player-to-player.
                // `says`/`shouts`/`auctions` are excluded on purpose --
                // NPCs use `says` too, so it proves nothing. See
                // docs/design/encounters.md, "Entity classification".
                if let (Some(who), Some(chan)) = (str_field("who"), str_field("chan")) {
                    let player_only = matches!(
                        chan.as_str(),
                        "tells you" | "tells the guild" | "tells the group" | "tell your party" | "tell the guild" | "tell the group"
                    );
                    if player_only {
                        self.encounters.entities.note_player_channel(&who);
                    }
                }
            }
            _ => {}
        }
    }

    /// Damage is what defines the encounter graph (`docs/design/encounters.md`:
    /// "each damage line is an edge"), so this is the only event kind that
    /// opens a new fight. Everything else attaches to whatever fight is
    /// already open, if any.
    fn record_damage(&mut self, ts: Millis, src: &str, dst: &str, ability: &str, tags: Tags, amount: u64, flags: Flags) {
        let enc = self.link(ts, src, dst);
        let a = self.store.sym(src);
        let t = self.store.sym(dst);
        let ab = self.store.ability_id(ability, tags);
        let idx = self.store.push(ts, EventKind::Damage, a, t, ab, amount, flags, enc.0);
        self.store.extend_encounter(enc, idx);
        self.drain_closed();
    }

    fn record_heal(&mut self, ts: Millis, src: &str, dst: &str, ability: &str, amount: u64) {
        let enc = self.current_encounter_of(src).or_else(|| self.current_encounter_of(dst));
        let a = self.store.sym(src);
        let t = self.store.sym(dst);
        let ab = self.store.ability_id(ability, tag::HEAL);
        let idx = self.store.push(ts, EventKind::Heal, a, t, ab, amount, 0, enc.map(|e| e.0).unwrap_or(NO_ENCOUNTER));
        if let Some(id) = enc {
            self.store.extend_encounter(id, idx);
        }
    }

    fn record_miss(&mut self, ts: Millis, src: &str, dst: &str) {
        let enc = self.current_encounter_of(src).or_else(|| self.current_encounter_of(dst));
        let a = self.store.sym(src);
        let t = self.store.sym(dst);
        let ab = self.store.ability_id("Miss", tag::MELEE);
        let idx = self.store.push(ts, EventKind::Miss, a, t, ab, 0, 0, enc.map(|e| e.0).unwrap_or(NO_ENCOUNTER));
        if let Some(id) = enc {
            self.store.extend_encounter(id, idx);
        }
    }

    fn record_death(&mut self, ts: Millis, victim: &str) {
        self.encounters.death(ts, victim);
        self.drain_closed();
    }

    /// Routes one damage edge through the encounter graph, then resolves it
    /// to a store `EncounterId`, opening one the first time this graph
    /// component is seen.
    fn link(&mut self, ts: Millis, actor: &str, target: &str) -> EncounterId {
        let enc_id = self.encounters.damage(ts, actor, target);
        if let Some(&id) = self.enc_map.get(&enc_id) {
            return id;
        }
        // Anchor the store encounter's single display label on whichever
        // side isn't the player -- "an armadillo", not "You". `open_encounter`
        // only needs the row index this event *will* get, which is exactly
        // the store's current length before the push that follows.
        let anchor = if target.eq_ignore_ascii_case("you") { actor } else { target };
        let sym = self.store.sym(anchor);
        let idx_hint = self.store.len() as u32;
        let id = self.store.open_encounter(sym, ts, idx_hint);
        self.enc_map.insert(enc_id, id);
        id
    }

    fn current_encounter_of(&self, name: &str) -> Option<EncounterId> {
        let enc_id = self.encounters.encounter_of(name)?;
        self.enc_map.get(&enc_id).copied()
    }

    /// Syncs newly-closed graph encounters into the store. `Builder::closed`
    /// only grows, so this drains what's new since the last call.
    fn drain_closed(&mut self) {
        while self.closed_seen < self.encounters.closed.len() {
            let c = &self.encounters.closed[self.closed_seen];
            if let Some(&store_id) = self.enc_map.get(&c.id) {
                self.store.close_encounter(store_id, c.end_ms, !c.slain.is_empty());
            }
            self.closed_seen += 1;
        }
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
