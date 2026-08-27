//! why: parses each line once and routes it into the store and the
//! encounter graph -- the "parsed db", nothing here ever reclassifies a line.
//!
//! Bridges two encounter models that intentionally differ:
//! `eqlp_store::Encounter` (a range with a single target label) and
//! `eqlp_session::graph::Builder` (a connected-component fight over many
//! entities). The translation is ingestion glue, lives here not either crate.

use crate::history::ParseRecord;
use crate::teleportdata;
use eqlp_core::coverage::{ShapeStat, DEFAULT_SHAPE_CAP};
use eqlp_core::event::Match;
use eqlp_core::shape::{ShapeMode, Shaper};
use eqlp_core::{field, Engine, Outcome};
use eqlp_session::{
    Allegiance, Builder, CastOutcome, CastResolver, ClassDetector, EncId, Kind, Policy, Spans,
    State, Timeline,
};
use eqlp_source::{Clock, Millis, VirtualClock};
use eqlp_store::{
    by_ability, by_actor, flag, score_parse, tag, EncounterId, EventKind, Filter, Flags,
    GearModifiers, Store, Sym, Tags, NO_ENCOUNTER,
};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::OnceLock;

/// why: reset whenever the tail target changes (new file, truncation, replacement)
#[derive(Debug, Clone, Default, Serialize)]
pub struct LineCounts {
    pub total: u64,
    pub matched: u64,
    pub unmatched: u64,
    pub headerless: u64,
    pub blank: u64,
    pub by_kind: BTreeMap<String, u64>,
}

/// why: same cap the `eqlp coverage` CLI tunes -- same clustering, live not offline.
/// An already-tracked shape keeps counting past the cap; only new ones drop, into `unmatched_shapes_overflow`.
const UNMATCHED_SHAPE_CAP: usize = DEFAULT_SHAPE_CAP;

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

/// why: bounds memory if the UI is slow to drain; frontend keeps its own smaller window on top
const MAX_PENDING_RECENT: usize = 500;

/// why: 8s window matching a summon to its Inner-Fire self-buff caster --
/// measured against the 2M-line reference log, resolved dozens of real
/// pets with no implausible pairing on manual spot check
const PET_MATCH_WINDOW_MS: Millis = 8_000;

/// why: 5s, generous against real quickbuff bursts -- landing lines resolve
/// within the same or next couple log-seconds after activation
const QUICKBUFF_WINDOW_MS: Millis = 5_000;

/// why: 3s cross-entity window proving a group cast, not coincidence --
/// confirmed against a real false positive (a group buff landed on 4
/// people 3s after the player's own Quick Buff activation)
const GROUP_CAST_WINDOW_MS: Millis = 3_000;

/// why: 30s, cast time + loading screen -- confirmed ~15s real Translocate
/// total; interrupted Gate casts never produce a zone.enter at all
const TELEPORT_WINDOW_MS: Millis = 30_000;

/// why: 15s, catches a maintained-buff pulse (measured real ~6s cadence)
/// vs a one-shot Quick Buff proc -- the dominant real false-positive shape
const PULSE_WINDOW_MS: Millis = 15_000;

/// why: safety net, far looser than the graph layer's own 10s idle close --
/// only catches what slips past that normal path
const STALE_ENCOUNTER_MS: Millis = 5 * 60 * 1000;

/// why: effective (account) level over time, from level.up lines,
/// first-person only. The effective level, not any one class's own --
/// swapping a class drops it silently with no line marking the drop
/// (confirmed real: 2->50 over 5 days, swap drops to 14, climbs to 36,
/// drops to 11). A single "current level" number would misrepresent this
/// -- `at` answers "as of this instant" so callers build a real range.
#[derive(Debug, Clone, Default)]
pub struct Levels {
    /// why: log-time order, `observe` trusts this and doesn't re-sort
    at_ts: Vec<(Millis, u8)>,
}

/// One AA rank purchase, in the order the log reports it.
#[derive(Debug, Clone)]
pub struct AaGrant {
    pub name: String,
    /// why: 1 for aa.gained (that line is rank 1), parsed rank for aa.improved
    pub rank: u8,
    /// why: points spent this rank; 0 for a free first rank, which several real AAs have
    pub cost: u32,
}

/// why: append-only log, no interpretation, same shape `Levels` uses.
/// Catalog enrichment is a separate lookup, kept apart from this raw record.
#[derive(Debug, Clone, Default)]
pub struct AaLog {
    at_ts: Vec<(Millis, AaGrant)>,
}

impl AaLog {
    pub fn observe(&mut self, ts: Millis, name: String, rank: u8, cost: u32) {
        self.at_ts.push((ts, AaGrant { name, rank, cost }));
    }

    /// why: every grant, in log-time order
    pub fn all(&self) -> impl Iterator<Item = &(Millis, AaGrant)> {
        self.at_ts.iter()
    }

    /// why: total spent; a free rank contributes 0, same as the log itself
    pub fn total_spent(&self) -> u32 {
        self.at_ts.iter().map(|(_, g)| g.cost).sum()
    }
}

/// why: highest live rank observed cast this session, "You" only, keyed
/// by base spell name -- confirmed real (2,131 "Ice Comet X" lines
/// alone, rank climbing over time). Distinct from the catalog name and
/// from base_spell_name's rank-stripping. Session-only, no persistence.
#[derive(Debug, Clone, Default)]
pub struct SpellRanks {
    best: HashMap<String, (u8, Millis)>,
}

impl SpellRanks {
    /// why: only keeps the highest rank seen; a lower re-observation is a no-op
    pub fn observe(&mut self, ts: Millis, base: &str, rank: u8) {
        match self.best.get_mut(base) {
            Some(e) if rank >= e.0 => *e = (rank, ts),
            Some(_) => {}
            None => {
                self.best.insert(base.to_string(), (rank, ts));
            }
        }
    }

    pub fn rank_of(&self, base: &str) -> Option<u8> {
        self.best.get(base).map(|(r, _)| *r)
    }

    /// why: every observed spell this session, unordered
    pub fn all(&self) -> impl Iterator<Item = (&str, u8)> {
        self.best.iter().map(|(k, (r, _))| (k.as_str(), *r))
    }
}

/// why: every exaltation-proc combat line this session, keyed by item
/// name. Exists because neither the inventory dump nor any log line
/// reveals what's socketed -- a proc firing is the only confirmable fact
/// (an earlier attempt inferring the effect from adjacent lines was
/// statistically meaningless, 85% of casts precede some shimmer line).
/// Only ever answers "has it fired, how many times", never "with what effect".
#[derive(Debug, Clone, Default)]
pub struct ExaltationProcs {
    counts: HashMap<String, u32>,
    first_seen_ms: HashMap<String, Millis>,
}

impl ExaltationProcs {
    pub fn observe(&mut self, ts: Millis, item: String) {
        *self.counts.entry(item.clone()).or_insert(0) += 1;
        self.first_seen_ms.entry(item).or_insert(ts);
    }

    /// why: 0 for never-fired, indistinguishable from genuinely no proc
    pub fn count(&self, item: &str) -> u32 {
        self.counts.get(item).copied().unwrap_or(0)
    }

    pub fn first_seen_ms(&self, item: &str) -> Option<Millis> {
        self.first_seen_ms.get(item).copied()
    }
}

/// why: two-tier confidence from EQL's begin/finish line pairs -- real
/// for both scribing (596/593 in the reference log) and memorizing.
/// Known: a finish landed at least once, definitive. Possible: a begin
/// with no finish (interrupted, or log ends mid-action). Known is sticky,
/// never downgrades. Deduped to first-seen, not full history -- memorize
/// can repeat hundreds of times as gems swap, only "known vs possible" matters.
#[derive(Debug, Clone, Default)]
pub struct SpellLog {
    entries: HashMap<String, SpellEvidence>,
}

#[derive(Debug, Clone, Copy)]
struct SpellEvidence {
    /// why: kept even after finished is set, so first_seen doesn't jump forward
    first_began: Millis,
    /// why: Some is what makes a spell Known rather than merely Possible
    finished: Option<Millis>,
}

impl SpellLog {
    pub fn observe_began(&mut self, ts: Millis, name: String) {
        self.entries.entry(name).or_insert(SpellEvidence {
            first_began: ts,
            finished: None,
        });
    }

    pub fn observe_finished(&mut self, ts: Millis, name: String) {
        let e = self.entries.entry(name).or_insert(SpellEvidence {
            first_began: ts,
            finished: None,
        });
        if e.finished.is_none() {
            e.finished = Some(ts);
        }
    }

    /// why: every Known spell, name + first confirmed time, arbitrary order
    pub fn known(&self) -> impl Iterator<Item = (&str, Millis)> {
        self.entries
            .iter()
            .filter_map(|(k, e)| e.finished.map(|ts| (k.as_str(), ts)))
    }

    /// why: every Possible-only spell, name + when the attempt began
    pub fn possible(&self) -> impl Iterator<Item = (&str, Millis)> {
        self.entries
            .iter()
            .filter(|(_, e)| e.finished.is_none())
            .map(|(k, e)| (k.as_str(), e.first_began))
    }
}

impl Levels {
    pub fn observe(&mut self, ts: Millis, level: u8) {
        self.at_ts.push((ts, level));
    }

    /// why: every level.up strictly inside [start, end), never a stale
    /// carried-over value -- a config-swap drop is real and silent, so
    /// `at(start)` would mix in a different configuration's level
    pub fn between(&self, start: Millis, end: Option<Millis>) -> impl Iterator<Item = u8> + '_ {
        let from = self.at_ts.partition_point(|&(t, _)| t < start);
        let to = match end {
            Some(e) => self.at_ts.partition_point(|&(t, _)| t < e),
            None => self.at_ts.len(),
        };
        self.at_ts[from..to].iter().map(|&(_, l)| l)
    }

    /// why: most recent level.up at or before ts; None before the first ever seen
    pub fn at(&self, ts: Millis) -> Option<u8> {
        let i = self.at_ts.partition_point(|&(t, _)| t <= ts);
        if i == 0 {
            None
        } else {
            Some(self.at_ts[i - 1].1)
        }
    }

    /// why: "as of now", not "as of when"; None mostly means "same level the whole file"
    pub fn latest(&self) -> Option<u8> {
        self.at_ts.last().map(|&(_, l)| l)
    }

    /// why: latest's own timestamp, for measuring XP progress within the current level
    pub fn latest_ts(&self) -> Option<Millis> {
        self.at_ts.last().map(|&(t, _)| t)
    }
}

/// why: a ping not an interval -- no companion "wears off" line exists
/// (checked directly, only poison logs its own end)
#[derive(Debug, Clone)]
pub struct EffectPing {
    pub ts: Millis,
    pub text: String,
    /// why: real, resolved caster name -- see `Ingest::attribute_effect`'s
    /// own doc for how this gets filled in; `None` is an honest "couldn't
    /// tell", not a guess
    pub source: Option<String>,
    /// why: real spell name explaining `text`, independent of whether
    /// `source` also resolved -- a spell can be identified even when
    /// exactly who cast it can't be (0 or 2+ real candidates nearby)
    pub skill: Option<String>,
    /// why: false for a resisted-cast ping (Skill Tracker's target-effects
    /// section) -- everything else pushed through here is a real landing,
    /// see `Effects::push`'s own doc
    pub landed: bool,
}

/// why: separate from Timeline's single exclusive State -- buffs stack,
/// no one "current state" to query, only "what landed recently".
#[derive(Debug, Clone, Default)]
pub struct Effects {
    by_entity: HashMap<u32, Vec<EffectPing>>,
}

impl Effects {
    /// why: inserted in timestamp order, same safety Timeline::push uses.
    /// `landed` -- see EffectPing's own doc; every call site before the
    /// Skill Tracker's target-effects section existed was a real landing.
    fn push(
        &mut self,
        entity: u32,
        ts: Millis,
        text: String,
        source: Option<String>,
        skill: Option<String>,
        landed: bool,
    ) {
        let v = self.by_entity.entry(entity).or_default();
        let at = v.partition_point(|p| p.ts <= ts);
        v.insert(
            at,
            EffectPing {
                ts,
                text,
                source,
                skill,
                landed,
            },
        );
    }

    /// why: trailing-window snapshot like dps_window, not a claim the effect is still active
    pub fn recent(&self, entity: u32, ts: Millis, window_ms: Millis) -> Vec<&EffectPing> {
        let Some(v) = self.by_entity.get(&entity) else {
            return Vec::new();
        };
        let from = ts - window_ms;
        let a = v.partition_point(|p| p.ts < from);
        let b = v.partition_point(|p| p.ts <= ts);
        v[a..b].iter().collect()
    }

    /// why: text-only convenience over `recent` for call sites that don't
    /// need source/skill attribution -- mainly this file's own tests,
    /// most of which predate attribute_effect and only ever asserted on text
    #[cfg(test)]
    fn recent_text(&self, entity: u32, ts: Millis, window_ms: Millis) -> Vec<&str> {
        self.recent(entity, ts, window_ms)
            .into_iter()
            .map(|p| p.text.as_str())
            .collect()
    }

    /// why: whole history not one instant's window, mirrors Timeline::transitions_of
    pub fn all(&self, entity: u32) -> &[EffectPing] {
        self.by_entity
            .get(&entity)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// why: Skill Tracker's target-effects fallback -- real bug, caught
    /// live: a pure debuff/CC cast (Tashania, a resist-decrease debuff
    /// with no damage component at all) never lands a single Damage
    /// event, so it never opens or extends combat::current_encounter's
    /// own damage-graph (deliberately damage-only, see record_damage's
    /// own doc) -- a support/CC character who never personally lands
    /// damage on the pull could never get a target-effects panel at
    /// all. This is the "who did my last real spell effect actually go
    /// against" answer instead, entity-agnostic (scans every entity's
    /// own most recent ping, not one already-known target).
    pub fn most_recent_by_you(&self) -> Option<(u32, &EffectPing)> {
        self.by_entity
            .iter()
            .filter_map(|(&entity, pings)| {
                pings
                    .iter()
                    .rev()
                    .find(|p| {
                        p.source
                            .as_deref()
                            .is_some_and(|s| s.eq_ignore_ascii_case("you"))
                    })
                    .map(|p| (entity, p))
            })
            .max_by_key(|(_, p)| p.ts)
    }
}

/// why: one chat line, whichever channel it landed in
#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub ts: Millis,
    /// why: the real sender -- "You" for the player's own outgoing line,
    /// same convention combat rows already use for src/dst
    pub who: String,
    pub text: String,
}

/// why: Guild/Party/Raid are shared channels, no partner to key on;
/// Pm's `with` always names the *other* side regardless of direction --
/// "X tells you" and "You told X" both key on X, so a PM thread reads
/// as one real conversation instead of splitting by who sent which line
#[derive(Debug, Clone)]
pub enum ChatChannel {
    Guild,
    Party,
    Raid,
    Pm { with: String },
}

/// why: kept whole-session like Store, not windowed like Effects -- chat
/// is meant to be browsed, not just checked "recently"
#[derive(Debug, Clone, Default)]
pub struct ChatLog {
    guild: Vec<ChatMessage>,
    party: Vec<ChatMessage>,
    raid: Vec<ChatMessage>,
    /// why: keyed by the other party's name -- see ChatChannel::Pm's own doc
    pm: HashMap<String, Vec<ChatMessage>>,
}

impl ChatLog {
    pub fn push(&mut self, ts: Millis, who: String, channel: ChatChannel, text: String) {
        let msg = ChatMessage { ts, who, text };
        match channel {
            ChatChannel::Guild => self.guild.push(msg),
            ChatChannel::Party => self.party.push(msg),
            ChatChannel::Raid => self.raid.push(msg),
            ChatChannel::Pm { with } => self.pm.entry(with).or_default().push(msg),
        }
    }

    pub fn guild(&self) -> &[ChatMessage] {
        &self.guild
    }
    pub fn party(&self) -> &[ChatMessage] {
        &self.party
    }
    pub fn raid(&self) -> &[ChatMessage] {
        &self.raid
    }
    /// why: empty slice for an unknown/never-messaged player, not an error
    pub fn pm_history(&self, player: &str) -> &[ChatMessage] {
        self.pm.get(player).map(Vec::as_slice).unwrap_or(&[])
    }
    /// why: one row per real PM partner, caller sorts by its own criteria
    /// (most-recent-first for the player list) -- this just hands back
    /// the raw facts: name plus that thread's own last message
    pub fn pm_threads(&self) -> impl Iterator<Item = (&str, &ChatMessage)> {
        self.pm
            .iter()
            .filter_map(|(name, msgs)| msgs.last().map(|m| (name.as_str(), m)))
    }
}

/// why: one real "X began casting Y" sighting, kept just long enough to
/// explain a later landing/wears-off line -- see `Ingest::attribute_effect`.
#[derive(Debug, Clone)]
struct RecentCast {
    ts: Millis,
    caster: u32,
    /// why: rank-stripped, matches `spelldata::spell_by_name`'s own base-name keying
    spell: String,
}

/// why: 35s -- the real catalog's own slowest cast (30s, confirmed via a
/// full scan of packs/spells.json) plus ATTRIBUTION_TOLERANCE_MS's own
/// slack, so a real slow cast's landing is never pruned away before it
/// can be checked against its own caster
const RECENT_CAST_RETENTION_MS: Millis = 35_000;

/// why: real per-entity "who's recently been casting" log, all entities
/// (not just "You" -- unlike classdetect's own pet exclusion, a pet's
/// real cast is real information here, not misleading class evidence)
#[derive(Debug, Clone, Default)]
struct RecentCasts {
    entries: Vec<RecentCast>,
}

impl RecentCasts {
    fn push(&mut self, ts: Millis, caster: u32, spell: String) {
        self.entries
            .retain(|e| ts - e.ts <= RECENT_CAST_RETENTION_MS);
        self.entries.push(RecentCast { ts, caster, spell });
    }
}

/// why: recovers a third-person landing's first-person key --
/// "<Name>'s <tail>." -> "Your <tail>.". A single mechanical transform,
/// safe because the dictionary lookup gates it: checked against 76,986
/// real possessive-shaped lines, recovers 35 real spell families with no
/// false positives -- a non-match just fails to reconstruct a real key.
/// Only the first "'s " is the boundary -- stylized names use a backtick, not apostrophe.
fn third_person_flavor(text: &str) -> Option<(String, String)> {
    let idx = text.find("'s ")?;
    if !text.ends_with('.') {
        return None;
    }
    let (who, rest) = text.split_at(idx);
    if who.is_empty() {
        return None;
    }
    let tail = &rest[3..];
    let candidate = format!("Your {tail}");
    if crate::flavordata::classes_for_flavor(&candidate).is_empty() {
        return None;
    }
    Some((who.to_string(), candidate))
}

/// why: 3rd-person conjugation for verb_suffix_table; only the two real
/// irregulars ("are"/"have") special-cased, else the regular rule. Not a
/// general conjugator -- a bad guess just fails the dictionary lookup downstream.
fn conjugate_third_person(verb: &str) -> Option<String> {
    if !verb.bytes().all(|b| b.is_ascii_lowercase()) {
        return None;
    }
    Some(match verb {
        "are" => "is".to_string(),
        "have" => "has".to_string(),
        v if v.ends_with(['s', 'x', 'z'])
            || v.ends_with("sh")
            || v.ends_with("ch")
            || v.ends_with('o') =>
        {
            format!("{v}es")
        }
        v if v.ends_with('y')
            && v.len() > 1
            && !matches!(v.as_bytes()[v.len() - 2], b'a' | b'e' | b'i' | b'o' | b'u') =>
        {
            format!("{}ies", &v[..v.len() - 1])
        }
        v => format!("{v}s"),
    })
}

/// why: a few third-person landings that aren't a grammatical transform,
/// the game shortens the sentence itself -- confirmed individually, named
/// exceptions rather than forced rules
const THIRD_PERSON_VERB_ALIASES: &[(&str, &str)] = &[
    ("combusts.", "You feel your skin combust."),
    // why: 517 real occurrences, reworded ("let forth" -> "lets loose") not conjugated
    (
        "lets loose a mighty yaulp.",
        "You feel a surge of strength as you let forth a mighty yaulp.",
    ),
];

/// why: third-person suffix -> first-person key, built once. Every rule
/// checked against the reference log's real unmatched backlog before
/// being trusted:
/// - "You <verb> <tail>." -> "<verb+s> <tail>." (79 real hits)
/// - "Your <noun> <verb> <tail>." -> "<verb+s> <tail>." (1 real spell)
/// - "You feel your <noun> <verb>..." -> "'s <noun> <verb+s>..." --
///   keeps the possessed noun, unlike THIRD_PERSON_VERB_ALIASES' combust (4 real hits)
/// - trailing " you." also gets a " them." sibling (3 real hits)
/// - "You feel <word>." (single adjective) also gets "looks <word>." (14 real hits)
/// - "You feel <tail>." (any length) also gets "is <tail>." (8 real hits)
fn verb_suffix_table() -> &'static HashMap<String, &'static str> {
    static TABLE: OnceLock<HashMap<String, &'static str>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table = HashMap::new();
        for key in crate::flavordata::all_texts() {
            let rest = match key.strip_prefix("You ") {
                Some(r) => r,
                None => continue,
            };
            if let Some((verb, tail)) = rest.split_once(' ') {
                if let Some(conj) = conjugate_third_person(verb) {
                    table.insert(format!("{conj} {tail}"), key);
                    // why: two separate substitute-verb families off "feel", see fn doc
                    if verb == "feel" {
                        if !tail.contains(' ') {
                            table.insert(format!("looks {tail}"), key);
                        }
                        table.insert(format!("is {tail}"), key);
                    }
                }
            }
            // why: noun-keeping sibling of the plain "You <verb> <tail>." rule above
            if let Some(rest) = key.strip_prefix("You feel your ") {
                if let Some((noun, verb_tail)) = rest.split_once(' ') {
                    let (verb, tail) = match verb_tail.split_once(' ') {
                        Some((v, t)) => (v, Some(t)),
                        None => (verb_tail.strip_suffix('.').unwrap_or(verb_tail), None),
                    };
                    if let Some(conj) = conjugate_third_person(verb) {
                        let suffix = match tail {
                            Some(t) => format!("'s {noun} {conj} {t}"),
                            None => format!("'s {noun} {conj}."),
                        };
                        table.insert(suffix, key);
                    }
                }
            }
        }
        for key in crate::flavordata::all_texts() {
            let rest = match key.strip_prefix("Your ") {
                Some(r) => r,
                None => continue,
            };
            let mut parts = rest.splitn(3, ' ');
            let (Some(_noun), Some(verb), Some(tail)) = (parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            if let Some(conj) = conjugate_third_person(verb) {
                table.insert(format!("{conj} {tail}"), key);
            }
        }
        // why: collected separately and inserted after, can't see its own output
        let them_variants: Vec<(String, &'static str)> = table
            .iter()
            .filter_map(|(suffix, &key)| {
                suffix
                    .strip_suffix(" you.")
                    .map(|base| (format!("{base} them."), key))
            })
            .collect();
        table.extend(them_variants);
        for &(suffix, key) in THIRD_PERSON_VERB_ALIASES {
            table.insert(suffix.to_string(), key);
        }
        table
    })
}

/// why: recovers a landing that differs by ordinary verb conjugation, not
/// a bare possessive. Two split points tried at every occurrence (an
/// entity name can be multi-word): plain space against verb_suffix_table's
/// verb entries, and "'s " against its noun-keeping entries. Safe like
/// `third_person_flavor` -- a wrong split just fails the lookup. Checked
/// against the real log: recovers 79 spell families, tens of thousands of lines.
fn verb_conjugated_flavor(text: &str) -> Option<(String, String)> {
    if !text.ends_with('.') {
        return None;
    }
    let table = verb_suffix_table();
    for (i, b) in text.bytes().enumerate() {
        if b != b' ' || i == 0 {
            continue;
        }
        let who = &text[..i];
        let tail = &text[i + 1..];
        if let Some(&key) = table.get(tail) {
            return Some((who.to_string(), key.to_string()));
        }
    }
    for (idx, _) in text.match_indices("'s ") {
        if idx == 0 {
            continue;
        }
        let who = &text[..idx];
        let tail = &text[idx..]; // includes the leading "'s "
        if let Some(&key) = table.get(tail) {
            return Some((who.to_string(), key.to_string()));
        }
    }
    None
}

/// why: Quick Buff class evidence waiting out its cancellation window
struct PendingQuickbuffEvidence {
    ts: Millis,
    who: u32,
    classes: &'static [String],
    text: String,
}

/// why: one not-yet-attributed Xp row -- exists instead of a loot-style search
struct PendingXp {
    /// why: index into Store columns; enc[row] gets backfilled once a matching death resolves this
    row: u32,
    ts: Millis,
}

/// why: everything parsed from one tailed file, plus the machinery
/// turning raw matches into store rows and encounters. One per tailed
/// file -- row indices and encounter ids are meaningless across a
/// different file. Doesn't own the Engine/Matcher -- the caller
/// classifies, this just routes the result.
pub struct Ingest {
    pub store: Store,
    pub encounters: Builder,
    pub zone: Spans,
    /// why: raw label -> wiki zone, resolved once per distinct label not per query
    wiki_zone_cache: HashMap<Sym, Option<&'static str>>,
    /// why: lowercased mob name -> "which encounter am I currently
    /// looting"; see `recent_encounter_for` for how this + loot_claimed match kill order
    loot_cursor: HashMap<String, EncounterId>,
    /// why: encounters already claimed by loot -- lets recent_encounter_for advance, not reuse
    loot_claimed: HashSet<EncounterId>,
    /// why: most recent not-yet-attributed Xp row -- Xp is emitted before
    /// its death line, nothing to search for yet; record_death resolves this
    pending_xp: Option<PendingXp>,
    /// why: very first timestamp processed, session_start's fallback; set once
    first_ts: Option<Millis>,
    /// why: AFK as of the most recent line; surfaced by currently_afk for Overview
    afk_state: bool,
    /// why: most recent afk.off timestamp, session_start's preferred answer over first_ts
    last_afk_off: Option<Millis>,
    /// why: session-wide entity states, keyed by the same Sym the store uses
    pub timeline: Timeline,
    /// why: per-cast outcome, keyed on interned Syms reused from store.names
    casts: CastResolver,
    /// why: per-entity class evidence grouped by zone visit, never reset;
    /// pub so combat.rs reads it directly, not through a wrapper
    pub classes: ClassDetector,
    /// why: effective account level over time, from level.up only
    pub levels: Levels,
    /// why: recognized buff landings on "You", separate log from timeline/State
    pub effects: Effects,
    /// why: Guild/Party/Raid/PM message history -- the Social tab's whole data source
    pub chat: ChatLog,
    /// why: private -- attribute_effect's own scratch data, nothing outside this file reads it directly
    recent_casts: RecentCasts,
    /// why: most recent /loc reading, raw coordinate order the line
    /// printed. Map files use a different order: confirmed (x,y) =
    /// (-y,-x) of this reading via brute-force against 9 real Lower Guk
    /// readings, avg 9.3 units off. Rare, a snapshot not continuous tracking.
    pub last_loc: Option<(Millis, f64, f64, f64)>,
    /// why: timestamp + landing of "You" or a proven ally's most recent
    /// teleport cast -- an ally's cast counts too, group-shaped
    /// Translocate/Circle lands the whole group. Not scoped to a visit,
    /// only read within TELEPORT_WINDOW_MS by entered_via_teleport.
    last_teleport_cast: Option<(Millis, teleportdata::TeleportLanding)>,
    /// why: exact landing the current visit was entered via, set on every
    /// Action::Zone; timestamped so consumers compare against last_loc
    /// and take whichever is fresher -- fixes a real bug where /loc used
    /// to win unconditionally even when stale
    pub entered_via_teleport: Option<(Millis, teleportdata::TeleportLanding)>,
    /// why: "You" only, unlike last_teleport_cast -- Origin's own
    /// description is personal not group-shaped
    last_origin_cast: Option<Millis>,
    /// why: timestamp + raw zone of the most recent real confirmation of
    /// where Origin sends this character -- genuinely dynamic, no
    /// wiki-quotable destination (confirmed: 4 different real zones over
    /// 3 weeks). Learned empirically, last-one-wins, self-correcting.
    pub learned_origin: Option<(Millis, String)>,
    /// why: overlay's timed-effects tracker -- see effects.rs's own doc.
    /// Most-recent-wins, self-only for invis/hide/sneak; charm is keyed
    /// by `who` so an unrelated spell's own wear-off (state.charm_broken's
    /// pattern is generic, fires for any spell) can't false-clear it.
    pub charm: Option<crate::effects::CharmStatus>,
    pub invis: Option<crate::effects::InvisStatus>,
    pub hide: Option<crate::effects::MomentaryStatus>,
    pub sneak: Option<crate::effects::MomentaryStatus>,
    /// why: overlay's Skill Tracker widget -- see skilltracker.rs's own doc
    pub skills: std::collections::HashMap<String, crate::skilltracker::SkillTrack>,
    /// why: every AA rank purchase this session, see AaLog
    pub aa: AaLog,
    /// why: every spell confirmed known this session, see SpellLog
    pub spellbook: SpellLog,
    /// why: highest live rank observed cast this session, "You" only, see SpellRanks
    pub spell_ranks: SpellRanks,
    /// why: every exaltation-proc line this session, see ExaltationProcs
    pub exaltation_procs: ExaltationProcs,
    enc_map: HashMap<EncId, EncounterId>,
    /// why: every entity per encounter, kept current as a fight grows --
    /// Store::Encounter only carries one label but a fight can hold several
    pub entities_by_enc: HashMap<EncounterId, Vec<String>>,
    /// why: how far into encounters.closed we've synced; Builder only appends, never drains
    closed_seen: usize,
    /// why: unresolved summon sightings waiting to match a new actor, pruned to PET_MATCH_WINDOW_MS
    pending_summons: Vec<(Millis, String)>,
    /// why: names ever seen acting, so pending_summons is only checked on an entity's first action
    seen_actors: HashSet<String>,
    /// why: resolved pet -> owner; checked by sym before interning so a
    /// matched pet's actions merge into the owner's identity
    pet_owner: HashMap<String, String>,
    /// why: encounters where "You" landed a confirmed hit on the anchor
    /// mob -- once inserted, future actors on that anchor promote inline
    you_confirmed_target_encs: HashSet<EncounterId>,
    /// why: open Quick Buff activation windows, resolved name -> activation timestamp
    pending_quickbuff: HashMap<String, Millis>,
    /// why: Quick Buff evidence held for GROUP_CAST_WINDOW_MS in case it's really a group cast
    pending_quickbuff_evidence: Vec<PendingQuickbuffEvidence>,
    /// why: every recognized flavor landing across entities, trailing
    /// GROUP_CAST_WINDOW_MS -- answers "did this land on someone else
    /// just now"; separate from effects (full history), pruned on every touch
    recent_flavor_landings: Vec<(Millis, u32, String)>,
    /// why: log-time clock, set from timestamps during replay, also
    /// advanced by real elapsed time once mark_live is called
    log_clock: VirtualClock,
    last_wall_ms: Option<Millis>,
    /// why: log_clock's value as of last_wall_ms; projecting from this
    /// pair (not a fresh read) avoids double-counting wall-elapsed time
    last_log_ms: Millis,
    live: bool,
    pub counts: LineCounts,
    /// why: unmatched-line shapes ranked by count, Debug's "Unparsed" tab;
    /// accumulated from both live tail and backfill so a fresh launch sees the whole picture
    unmatched_shapes: HashMap<Vec<u8>, ShapeStat>,
    unmatched_shapes_overflow: u64,
    shaper: Shaper,
    shape_scratch: Vec<u8>,
    pub recent: Vec<RecentLine>,
    /// why: notification-worthy events pending drain; pure data, no I/O,
    /// live-only so backfill doesn't fire a burst of sounds
    pub pending_notifications: Vec<crate::notifications::NotificationEvent>,
    /// why: one record per closed encounter pending drain, pure data, see crate::history
    pub pending_history: Vec<ParseRecord>,
    /// why: filenames from Outputfile Complete lines pending drain; only
    /// records that a dump exists, tail_worker.rs reads and acts on it
    pub pending_inventory_files: Vec<String>,
}

impl Default for Ingest {
    fn default() -> Self {
        Ingest {
            store: Store::default(),
            encounters: Builder::new(Policy::default()),
            zone: Spans::default(),
            wiki_zone_cache: HashMap::new(),
            loot_cursor: HashMap::new(),
            loot_claimed: HashSet::new(),
            pending_xp: None,
            first_ts: None,
            afk_state: false,
            last_afk_off: None,
            timeline: Timeline::default(),
            casts: CastResolver::default(),
            classes: ClassDetector::default(),
            levels: Levels::default(),
            effects: Effects::default(),
            chat: ChatLog::default(),
            recent_casts: RecentCasts::default(),
            last_loc: None,
            last_teleport_cast: None,
            entered_via_teleport: None,
            last_origin_cast: None,
            learned_origin: None,
            charm: None,
            invis: None,
            hide: None,
            sneak: None,
            skills: std::collections::HashMap::new(),
            aa: AaLog::default(),
            spellbook: SpellLog::default(),
            spell_ranks: SpellRanks::default(),
            exaltation_procs: ExaltationProcs::default(),
            enc_map: HashMap::new(),
            entities_by_enc: HashMap::new(),
            closed_seen: 0,
            pending_summons: Vec::new(),
            seen_actors: HashSet::new(),
            you_confirmed_target_encs: HashSet::new(),
            pet_owner: HashMap::new(),
            pending_quickbuff: HashMap::new(),
            pending_quickbuff_evidence: Vec::new(),
            recent_flavor_landings: Vec::new(),
            log_clock: VirtualClock::new(0),
            last_wall_ms: None,
            last_log_ms: 0,
            live: false,
            counts: LineCounts::default(),
            unmatched_shapes: HashMap::new(),
            unmatched_shapes_overflow: 0,
            shaper: Shaper::new(),
            shape_scratch: Vec::new(),
            recent: Vec::new(),
            pending_notifications: Vec::new(),
            pending_history: Vec::new(),
            pending_inventory_files: Vec::new(),
        }
    }
}

impl Ingest {
    /// why: log's own clock, ms, no timezone, same basis as every LocalTs in eqlp-core
    pub fn now_ms(&self) -> Millis {
        self.log_clock.now_ms()
    }

    /// why: most recent afk.off, else first_ts; a return from AFK reads
    /// as a fresh session on purpose -- AFK time inside an unbroken
    /// window would silently drag down plat/hour and xp/hour. Going AFK
    /// itself doesn't end anything, only coming back does.
    pub fn session_start(&self) -> Option<Millis> {
        self.last_afk_off.or(self.first_ts)
    }

    /// why: AFK as of the most recently processed line
    pub fn currently_afk(&self) -> bool {
        self.afk_state
    }

    /// why: tier as of ts, stamped onto every row so a baseline can scope
    /// to "this target, this difficulty" with a plain Filter
    fn current_tier(&self, ts: Millis) -> u8 {
        crate::zone::zone_tier(self.zone.at(ts).unwrap_or("")).1
    }

    /// why: current_tier's sibling, the zone itself interned; stamped
    /// once at encounter-open rather than re-derived per query. Also
    /// primes wiki_zone_cache eagerly, not lazy-on-first-query.
    fn current_zone(&mut self, ts: Millis) -> Option<Sym> {
        let z = self.zone.at(ts)?.to_string();
        let sym = self.sym(&z);
        self.resolved_wiki_zone(sym);
        Some(sym)
    }

    /// why: wiki zone id (not display name -- an exact == vs a
    /// case-insensitive compare, directly eyeball-checkable). Cached,
    /// runs at most once per distinct raw label a session ever sees.
    fn resolved_wiki_zone(&mut self, raw_zone: Sym) -> Option<&'static str> {
        if let Some(&cached) = self.wiki_zone_cache.get(&raw_zone) {
            return cached;
        }
        let resolved = {
            let raw = self.store.name(raw_zone);
            crate::zonedata::zones()
                .iter()
                .find(|z| crate::zone::zone_matches(raw, &z.name))
                .map(|z| z.id.as_str())
        };
        self.wiki_zone_cache.insert(raw_zone, resolved);
        resolved
    }

    /// why: read-only cache lookup; None ambiguous in theory but not in
    /// practice -- current_zone always resolves before an encounter can exist
    pub fn cached_wiki_zone(&self, raw_zone: Sym) -> Option<&'static str> {
        self.wiki_zone_cache.get(&raw_zone).copied().flatten()
    }

    /// why: switches from replaying-fast to live -- tick starts advancing
    /// the clock by real elapsed time too, not just new-line timestamps
    pub fn mark_live(&mut self) {
        self.live = true;
        self.last_wall_ms = None; // why: next tick sets the baseline, not a jump
    }

    /// why: live-path unmatched line, folded directly -- no per-thread copy to merge
    fn note_unmatched_shape(&mut self, text: &[u8]) {
        self.shaper
            .shape_into(text, ShapeMode::Aggressive, &mut self.shape_scratch);
        let key = self.shape_scratch.clone();
        if let Some(s) = self.unmatched_shapes.get_mut(&key) {
            s.count += 1;
        } else if self.unmatched_shapes.len() < UNMATCHED_SHAPE_CAP {
            self.unmatched_shapes.insert(
                key,
                ShapeStat {
                    count: 1,
                    example: text.to_vec(),
                },
            );
        } else {
            self.unmatched_shapes_overflow += 1;
        }
    }

    /// why: one already-shaped chunk result from backfill's parallel
    /// classify step; existing example kept on a hit, first-seen not last-merged
    fn merge_unmatched_shape(&mut self, shape: Vec<u8>, stat: ShapeStat) {
        if let Some(existing) = self.unmatched_shapes.get_mut(&shape) {
            existing.count += stat.count;
        } else if self.unmatched_shapes.len() < UNMATCHED_SHAPE_CAP {
            self.unmatched_shapes.insert(shape, stat);
        } else {
            self.unmatched_shapes_overflow += stat.count;
        }
    }

    /// why: highest count first, same ranking as `eqlp coverage`
    pub fn unmatched_shapes_top(&self, n: usize) -> Vec<(&[u8], &ShapeStat)> {
        let mut v: Vec<_> = self
            .unmatched_shapes
            .iter()
            .map(|(k, s)| (k.as_slice(), s))
            .collect();
        v.sort_unstable_by(|a, b| b.1.count.cmp(&a.1.count).then(a.0.cmp(b.0)));
        v.truncate(n);
        v
    }

    pub fn unmatched_shapes_distinct(&self) -> usize {
        self.unmatched_shapes.len()
    }

    /// why: a line count, not a shape count -- can be large even when the cap is only a little too small
    pub fn unmatched_shapes_overflow(&self) -> u64 {
        self.unmatched_shapes_overflow
    }

    /// why: called once per line, in order, with the already-computed classification
    pub fn route(&mut self, engine: &Engine, line: &[u8], outcome: &Outcome) {
        self.counts.total += 1;
        match outcome {
            Outcome::Matched(m) => {
                self.counts.matched += 1;
                let rule = engine.rule(m.rule);
                *self.counts.by_kind.entry(rule.kind.clone()).or_insert(0) += 1;
                let ts_ms = m.ts.secs() * 1000;

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
                    if let Some(notif) = crate::notifications::notification_for(
                        engine,
                        rule.id.as_str(),
                        m,
                        line,
                        ts_ms,
                    ) {
                        self.pending_notifications.push(notif);
                    }
                }

                self.log_clock.set_at_least(ts_ms);
                if let Some(action) = extract_action(engine, rule.id.as_str(), m, line) {
                    self.apply(ts_ms, action);
                }
            }
            Outcome::Unmatched { ts, body } => {
                self.counts.unmatched += 1;
                // why: checked against the flavor dictionary first,
                // unconditional -- a hit is understood, no business in "Unparsed"
                let ts_ms = ts.secs() * 1000;
                let text = String::from_utf8_lossy(body.slice(line));
                if !self.flavor_evidence_for(ts_ms, &text) {
                    self.note_unmatched_shape(body.slice(line));
                }
            }
            Outcome::Headerless { .. } => self.counts.headerless += 1,
            Outcome::Blank => self.counts.blank += 1,
        }
    }

    /// why: called once per worker tick, advances the log clock during
    /// idle stretches and closes quiet fights. Projects from last_log_ms,
    /// not a fresh read -- else wall-elapsed would double-count time
    /// lines already advanced the clock past, racing it ahead of real time.
    pub fn tick(&mut self, wall_now_ms: Millis) {
        if self.live {
            if let Some(last) = self.last_wall_ms {
                let elapsed = (wall_now_ms - last).max(0);
                self.log_clock.set_at_least(self.last_log_ms + elapsed);
            }
            self.last_wall_ms = Some(wall_now_ms);
            self.last_log_ms = self.log_clock.now_ms();
        }
        let now = self.log_clock.now_ms();
        self.encounters.expire(now);
        self.casts.expire(now);
        self.flush_cast_resolutions();
        self.drain_closed();
        self.store.close_stale_encounters(now, STALE_ENCOUNTER_MS);
    }

    /// why: executes one extracted action; never touches line/Match/Engine
    /// so the same logic runs from a sequential backfill merge or inline live
    fn apply(&mut self, ts: Millis, action: Action) {
        if self.first_ts.is_none() {
            self.first_ts = Some(ts);
        }
        match action {
            Action::Damage {
                src,
                dst,
                ability,
                tags,
                amount,
                flags,
            } => {
                self.record_damage(ts, &src, &dst, &ability, tags, amount, flags);
                // why: a resisted spell deals no damage, so damage is
                // unambiguous proof of landing; a no-op outside SPELL tags
                if tags & tag::SPELL != 0 {
                    let src_sym = self.sym(&src).0;
                    let spell_sym = self.store.sym(base_spell_name(&ability)).0;
                    self.casts.confirm_landed(ts, src_sym, spell_sym);
                }
            }
            Action::Heal {
                src,
                dst,
                ability,
                amount,
            } => {
                let dst = resolve_reflexive(&dst, &src);
                self.record_heal(ts, &src, &dst, &ability, amount);
                let src_sym = self.sym(&src).0;
                let spell_sym = self.store.sym(base_spell_name(&ability)).0;
                self.casts.confirm_landed(ts, src_sym, spell_sym);
            }
            Action::Miss {
                src,
                dst,
                verb,
                flags,
            } => self.record_avoided(ts, &src, &dst, &verb, flag::MISSED | flags),
            Action::Block {
                src,
                dst,
                verb,
                flags,
            } => self.record_avoided(ts, &src, &dst, &verb, flag::BLOCKED | flags),
            Action::Dodge {
                src,
                dst,
                verb,
                flags,
            } => self.record_avoided(ts, &src, &dst, &verb, flag::DODGED | flags),
            Action::Parry {
                src,
                dst,
                verb,
                flags,
            } => self.record_avoided(ts, &src, &dst, &verb, flag::PARRIED | flags),
            Action::Death { victim } => self.record_death(ts, &victim),
            Action::Zone { zone } => {
                // why: stop fights bleeding across zone changes
                self.encounters.close_all(ts);
                // why: a charmed pet never follows you across a zone line --
                // real loss even with no "spell has worn off" confirmation
                // line at all (charm's own break is often silent on zoning)
                if let Some(c) = &mut self.charm {
                    if c.active {
                        c.active = false;
                        c.since_ms = ts;
                    }
                }
                self.entered_via_teleport = self
                    .last_teleport_cast
                    .clone()
                    .filter(|(cast_ts, _)| ts - cast_ts <= TELEPORT_WINDOW_MS)
                    .map(|(_, landing)| (ts, landing));
                // why: Origin's own real confirmation -- same window/shape
                // as the wiki-fixed teleports, recording which zone instead
                if self
                    .last_origin_cast
                    .is_some_and(|cast_ts| ts - cast_ts <= TELEPORT_WINDOW_MS)
                {
                    self.learned_origin = Some((ts, zone.clone()));
                }
                self.zone.enter(ts, zone);
            }
            Action::LevelUp { level } => {
                self.levels.observe(ts, level);
            }
            Action::AaGained { name, rank, cost } => {
                // why: AA grants are always first-person (the log never
                // shows another player's own AA gain) -- real, curated
                // class data from the wiki scrape (aadata.rs's own
                // `category` field), never wired into classdetect before
                let you = self.sym("You");
                self.classes.observe_cast(
                    you.0,
                    self.zone.index_at(ts),
                    &crate::aadata::classes_for(&name),
                );
                self.aa.observe(ts, name, rank, cost);
            }
            Action::SpellBegan { name } => {
                self.spellbook.observe_began(ts, name);
            }
            Action::SpellFinished { name } => {
                self.spellbook.observe_finished(ts, name);
            }
            Action::OutputfileComplete { file } => {
                self.pending_inventory_files.push(file);
            }
            Action::ExaltationProc { item } => {
                self.exaltation_procs.observe(ts, item);
            }
            Action::Cast { who, spell } => {
                // why: only "Inner Fire" specifically -- measured safe
                // against the real log; "any first cast" mismatched real
                // not-yet-proven players near a pet summon
                if spell == "Inner Fire" {
                    self.note_actor(ts, &who);
                }
                // why: "You" or a proven ally -- group-shaped teleports
                // land the whole group, an unproven stranger's cast doesn't count
                if who == "You" || self.is_ally(&who, ts) {
                    if let Some(landing) = teleportdata::landing_for(&spell) {
                        self.last_teleport_cast = Some((ts, landing));
                    }
                }
                // why: Origin is personal only, doesn't join the ally-aware check above
                if who == "You" && spell == "Origin" {
                    self.last_origin_cast = Some(ts);
                }
                // why: personal only -- for the player's own spellbook display
                if who == "You" {
                    if let (base, Some(rank)) = split_cast_rank(&spell) {
                        self.spell_ranks.observe(ts, base, rank);
                    }
                }
                // A cast line proves the ability isn't a weapon proc; no
                // store row needed, just the ability metadata.
                let id = self.store.ability_id(&spell, tag::SPELL);
                self.store.abilities.note_cast(id);
                let caster = self.sym(&who);
                self.clear_dead_if_acting(ts, caster);
                let base = base_spell_name(&spell);
                // why: Skill Tracker's own-cooldowns section -- spells are
                // observed off cast.begin, not any Damage event they might
                // also produce (see skilltracker.rs's own doc for why)
                if who == "You" {
                    crate::skilltracker::observe_skill_use(&mut self.skills, ts, base, true);
                }
                let spell_sym = self.store.sym(base).0;
                self.casts.begin(ts, caster.0, spell_sym);
                // why: every real caster, pets included -- attribute_effect's
                // own doc, unlike classdetect's pet exclusion, a pet's real
                // cast is real information here, not misleading class evidence
                self.recent_casts.push(ts, caster.0, base.to_string());
                if !self.is_pet(&who) {
                    self.classes.observe_cast(
                        caster.0,
                        self.zone.index_at(ts),
                        class_evidence_for(base),
                    );
                }
            }
            Action::CastResisted {
                source,
                spell,
                target,
            } => {
                let src = self.sym(&source).0;
                let spell_sym = self.store.sym(base_spell_name(&spell)).0;
                self.casts
                    .resolve(ts, src, spell_sym, CastOutcome::Resisted);
                // why: Skill Tracker's target-effects section -- a failed
                // attempt is still worth showing (flashed, 0:00), not
                // just silently dropped
                if let Some(target) = target {
                    let resolved = self.resolve_name(&target);
                    let sym = self.sym(&resolved).0;
                    self.effects
                        .push(sym, ts, spell.clone(), Some(source), Some(spell), false);
                }
            }
            Action::PetSpellWoreOff { spell } => {
                self.record_effect_ping(ts, "Your pet", &spell);
            }
            Action::CastInterrupted { source, spell } => {
                let src = self.sym(&source).0;
                let spell_sym = self.store.sym(base_spell_name(&spell)).0;
                self.casts
                    .resolve(ts, src, spell_sym, CastOutcome::Interrupted);
            }
            Action::CastFizzled { source, spell } => {
                let src = self.sym(&source).0;
                let spell_sym = self.store.sym(base_spell_name(&spell)).0;
                self.casts.resolve(ts, src, spell_sym, CastOutcome::Fizzled);
            }
            Action::CastBlocked {
                spell,
                target,
                blocker,
            } => {
                // why: no resolve() call -- "blocked by stacking conflict"
                // isn't the same failure as Resisted (a resist roll), would
                // skew resist-rate stats; no outcome variant fits, stays out entirely
                let you = self.sym("You").0;
                self.classes.observe_cast(
                    you,
                    self.zone.index_at(ts),
                    class_evidence_for(base_spell_name(&spell)),
                );
                if let Some(blocker) = blocker {
                    self.record_effect_ping(ts, &target, &blocker);
                }
            }
            Action::SpellOverwritten { spell, who } => {
                // why: a real, unambiguous landing -- see this Action's
                // own doc. No attribute_effect needed at all (source
                // and spell are both named directly in the line, not
                // inferred), same direct-push shape CastResisted's own
                // handler already uses, just landed: true this time.
                let you = self.sym("You").0;
                self.classes.observe_cast(
                    you,
                    self.zone.index_at(ts),
                    class_evidence_for(base_spell_name(&spell)),
                );
                let resolved = self.resolve_name(&who);
                let sym = self.sym(&resolved).0;
                self.effects.push(
                    sym,
                    ts,
                    spell.clone(),
                    Some("You".to_string()),
                    Some(spell.clone()),
                    true,
                );
                // why: also feeds Skill Tracker's own recovery-clock
                // tracking (skilltracker.rs's own doc), same as every
                // other real landing confirmation (record_damage's
                // tag::SPELL branch, Heal, confirm_spell_effect)
                let spell_sym = self.store.sym(base_spell_name(&spell)).0;
                self.casts.confirm_landed(ts, you, spell_sym);
            }
            Action::StateEffect { target, text } => self.record_effect_ping(ts, &target, &text),
            Action::PlayerLoc { x, y, z } => {
                self.last_loc = Some((ts, x, y, z));
            }
            Action::AbilityActivated { who, ability } => {
                let sym = self.sym(&who);
                if !self.is_pet(&who) {
                    self.classes.observe_cast(
                        sym.0,
                        self.zone.index_at(ts),
                        crate::classdata::classes_for(&ability),
                    );
                }
                self.record_effect_ping(ts, &who, &ability);
            }
            Action::PetSummon { owner } => self.note_pet_summon(ts, &owner),
            Action::Stance { stance } => {
                let you = self.sym("You");
                self.classes.observe_cast(
                    you.0,
                    self.zone.index_at(ts),
                    crate::stancedata::classes_for(&stance),
                );
            }
            Action::SkillUp { skill } => {
                let you = self.sym("You");
                self.classes.observe_cast(
                    you.0,
                    self.zone.index_at(ts),
                    crate::skilldata::classes_for(&skill),
                );
            }
            Action::Invocation { invocation } => {
                let you = self.sym("You");
                self.classes.observe_cast(
                    you.0,
                    self.zone.index_at(ts),
                    crate::invocationdata::classes_for(&invocation),
                );
            }
            Action::PlayerProof { who } => self.encounters.entities.note_player_channel(&who),
            Action::ChatMessage { who, channel, text } => {
                // why: same real-player evidence PlayerProof gave every
                // one of these channels before ChatMessage replaced it
                self.encounters.entities.note_player_channel(&who);
                self.chat.push(ts, who, channel, text);
            }
            Action::QuickBuff { who } => self.note_quickbuff(ts, &who),
            Action::Mez { who } => {
                let sym = self.sym(&who);
                self.timeline.observed(ts, sym.0, State::Mezzed);
            }
            Action::Charm { who } => {
                let sym = self.sym(&who);
                self.timeline.observed(ts, sym.0, State::Charmed);
                self.charm = Some(crate::effects::CharmStatus {
                    who,
                    active: true,
                    since_ms: ts,
                });
            }
            Action::Recovered { who } => {
                let sym = self.sym(&who);
                self.timeline.observed(ts, sym.0, State::Engaged);
                // why: state.charm_broken's own pattern is generic (any
                // spell wearing off of any target) -- only clear the
                // tracked charm if this is really that same target
                if let Some(c) = &mut self.charm {
                    if c.who == who && c.active {
                        c.active = false;
                        c.since_ms = ts;
                    }
                }
            }
            Action::InvisFading => {
                self.invis = Some(crate::effects::InvisStatus {
                    active: self.invis.map(|s| s.active).unwrap_or(true),
                    fading: true,
                    since_ms: ts,
                });
            }
            Action::InvisLanded => {
                self.invis = Some(crate::effects::InvisStatus {
                    active: true,
                    fading: false,
                    since_ms: ts,
                });
            }
            Action::InvisEnded => {
                self.invis = Some(crate::effects::InvisStatus {
                    active: false,
                    fading: false,
                    since_ms: ts,
                });
            }
            Action::HideSuccess => {
                self.hide = Some(crate::effects::MomentaryStatus {
                    outcome: crate::effects::MomentaryOutcome::Success,
                    since_ms: ts,
                });
            }
            Action::HideFailure => {
                self.hide = Some(crate::effects::MomentaryStatus {
                    outcome: crate::effects::MomentaryOutcome::Failure,
                    since_ms: ts,
                });
            }
            Action::HideEnded => {
                self.hide = Some(crate::effects::MomentaryStatus {
                    outcome: crate::effects::MomentaryOutcome::Ended,
                    since_ms: ts,
                });
            }
            Action::SneakSuccess => {
                self.sneak = Some(crate::effects::MomentaryStatus {
                    outcome: crate::effects::MomentaryOutcome::Success,
                    since_ms: ts,
                });
            }
            Action::SneakFailure => {
                self.sneak = Some(crate::effects::MomentaryStatus {
                    outcome: crate::effects::MomentaryOutcome::Failure,
                    since_ms: ts,
                });
            }
            Action::SneakEnded => {
                self.sneak = Some(crate::effects::MomentaryStatus {
                    outcome: crate::effects::MomentaryOutcome::Ended,
                    since_ms: ts,
                });
            }
            Action::Loot {
                item,
                corpse,
                qty,
                sold_for,
            } => {
                self.record_loot(ts, &item, &corpse, qty, sold_for.is_some());
                if let Some(text) = sold_for {
                    self.record_currency(ts, "autosell", &text);
                }
            }
            Action::Xp { scope, pct } => self.record_xp(ts, &scope, pct),
            Action::Currency { source, text } => self.record_currency(ts, &source, &text),
            Action::AfkOn => self.afk_state = true,
            Action::AfkOff => {
                self.afk_state = false;
                self.last_afk_off = Some(ts);
            }
        }
        self.flush_cast_resolutions();
    }

    /// why: pushes every finished cast judgment into the store as a Cast
    /// row; called after every action and once per tick to catch
    /// expiry-driven Unconfirmed closures. target = actor since cast.begin
    /// never names a real target in this log.
    fn flush_cast_resolutions(&mut self) {
        for r in self.casts.drain_resolved() {
            let actor = Sym(r.source);
            let spell_name = self.store.name(Sym(r.spell)).to_string();
            let ability = self.store.ability_id(&spell_name, tag::SPELL);
            let flags = match r.outcome {
                CastOutcome::Landed => flag::CAST_LANDED,
                CastOutcome::Resisted => flag::CAST_RESISTED,
                CastOutcome::Interrupted => flag::CAST_INTERRUPTED,
                CastOutcome::Fizzled => flag::CAST_FIZZLED,
                CastOutcome::Unconfirmed => flag::CAST_UNCONFIRMED,
            };
            let tier = self.current_tier(r.end_ms);
            // why: real bug, caught live -- this always pushed NO_ENCOUNTER,
            // so cast_rows()'s own `enc[i] != id.0` check could never match
            // a real encounter and CombatSummaryDto.casts was silently
            // empty for every selection, always. Same lookup record_avoided
            // already uses for Miss rows.
            let actor_name = self.store.name(actor).to_string();
            // why: Skill Tracker's own recovery clock -- Spencer's own
            // correction, a spell's real cooldown starts at this
            // confirmed landing, not at cast.begin's own attempt
            // timestamp; see skilltracker.rs's own doc
            if r.outcome == CastOutcome::Landed && actor_name.eq_ignore_ascii_case("you") {
                crate::skilltracker::observe_skill_landed(&mut self.skills, r.end_ms, &spell_name);
            }
            let enc = self.current_encounter_of(&actor_name);
            let idx = self.store.push(
                r.end_ms,
                EventKind::Cast,
                actor,
                actor,
                ability,
                0,
                flags,
                enc.map(|e| e.0).unwrap_or(NO_ENCOUNTER),
                tier,
            );
            if let Some(id) = enc {
                self.store.extend_encounter(id, idx);
            }
        }
    }

    /// why: damage is what defines the encounter graph, the only event kind that opens a new fight
    #[allow(clippy::too_many_arguments)] // why: each param is a distinct field off a real damage log line
    fn record_damage(
        &mut self,
        ts: Millis,
        src: &str,
        dst: &str,
        ability: &str,
        tags: Tags,
        amount: u64,
        flags: Flags,
    ) {
        // why: Spencer's own ask -- "when you charm something, and you see
        // no indication of charm ending, but you see outward combat on a
        // similarly named mob, that means its a new target". A charmed pet
        // can attack enemies all session without a break line ever needing
        // to fire, but it can never legitimately land a hit on "You" --
        // that alone is real proof the charm is already gone (naturally
        // expired, a fresh backfire, or a new mob reusing the same name
        // after the old one died), the same effective signal
        // Action::Recovered's own explicit break line gives, just inferred
        // here from real combat instead of waiting on one that may never
        // come. Same two-part clear Action::Recovered does -- self.charm
        // AND the timeline, so target_effects' own State::Charmed check
        // agrees immediately too.
        if let Some(c) = &mut self.charm {
            if c.active && c.who.eq_ignore_ascii_case(src) && dst.eq_ignore_ascii_case("you") {
                c.active = false;
                c.since_ms = ts;
                let sym = self.sym(src);
                self.timeline.observed(ts, sym.0, State::Engaged);
            }
        }
        let enc = self.link(ts, src, dst);
        let a = self.sym(src);
        let t = self.sym(dst);
        self.note_shared_target(ts, enc, src, t);
        self.clear_dead_if_acting(ts, a);
        // why: melee only -- a spell's own use is observed off cast.begin
        // instead (see Action::Cast's own handling), never both, or a
        // damage spell's own cast and its landing would count as two
        // separate "uses" milliseconds apart and corrupt the reuse gap
        if src.eq_ignore_ascii_case("you") && tags & tag::SPELL == 0 {
            crate::skilltracker::observe_skill_use(&mut self.skills, ts, ability, true);
        }
        let ab = self.store.ability_id(ability, tags);
        let tier = self.current_tier(ts);
        let idx = self
            .store
            .push(ts, EventKind::Damage, a, t, ab, amount, flags, enc.0, tier);
        self.store.extend_encounter(enc, idx);
        self.drain_closed();
    }

    /// why: "same mob damage as me" proves party membership, stronger and
    /// more common than chat evidence -- promotes via note_shared_target,
    /// same sticky-forever mechanism as chat proof. Two paths: the moment
    /// "You" confirms the fight, sweep back over everyone who already hit
    /// the anchor; after that, every future hit promotes inline. Never
    /// promotes the anchor hitting itself (reflected shield) or a currently
    /// charmed actor (would outlive the charm).
    fn note_shared_target(&mut self, ts: Millis, enc: EncounterId, src: &str, dst_sym: Sym) {
        let Some(anchor) = self.store.encounter(enc).map(|e| e.target) else {
            return;
        };
        if dst_sym != anchor {
            return; // why: damage to something other than this fight's own mob proves nothing
        }
        let src_resolved = self.resolve_name(src);
        if src_resolved.eq_ignore_ascii_case("you") {
            if self.you_confirmed_target_encs.insert(enc) {
                for (sym, ..) in
                    by_actor(&self.store, &Filter::encounter(enc).damage().target(anchor))
                {
                    if sym != anchor {
                        self.promote_party_member(sym, ts);
                    }
                }
            }
        } else if self.you_confirmed_target_encs.contains(&enc) {
            let sym = self.sym(&src_resolved);
            if sym != anchor {
                self.promote_party_member(sym, ts);
            }
        }
    }

    /// why: shared guard, never promotes a currently charmed entity -- a
    /// temporary ally must not become a permanent one. Spencer's own
    /// framing: real players stay a consistent, permanent classification
    /// (chat proof or shared-target proof); a mob is only ever a
    /// temporary ally (charm, already correctly time-scoped via the
    /// timeline's own State::Charmed -- reverts the instant the charm
    /// itself does, nothing sticky about it). Real bug, caught live: a
    /// mob dealing damage to an anchor "You" already confirmed (two
    /// mobs cross-tangled, cleave splash, ...) got promoted to
    /// permanent Kind::Player via this same path, silently poisoning
    /// EVERY later encounter with that same name -- "a haunted chest,
    /// only thing in combat... wasnt parsing to the ui for it". Guarded
    /// with plausible_player_name so an obviously-a-mob name can never
    /// earn that permanent status in the first place.
    fn promote_party_member(&mut self, sym: Sym, ts: Millis) {
        if matches!(self.timeline.state_at(sym.0, ts), Some((State::Charmed, _))) {
            return;
        }
        let name = self.store.name(sym).to_string();
        if !plausible_player_name(&name) {
            return;
        }
        self.encounters.entities.note_shared_target(&name);
    }

    fn record_heal(&mut self, ts: Millis, src: &str, dst: &str, ability: &str, amount: u64) {
        let enc = self
            .current_encounter_of(src)
            .or_else(|| self.current_encounter_of(dst));
        let a = self.sym(src);
        let t = self.sym(dst);
        self.clear_dead_if_acting(ts, a);
        let ab = self.store.ability_id(ability, tag::HEAL);
        let tier = self.current_tier(ts);
        let idx = self.store.push(
            ts,
            EventKind::Heal,
            a,
            t,
            ab,
            amount,
            0,
            enc.map(|e| e.0).unwrap_or(NO_ENCOUNTER),
            tier,
        );
        if let Some(id) = enc {
            self.store.extend_encounter(id, idx);
        }
    }

    /// why: a fully-avoided swing lands on the same ability row a landed
    /// swing would, tagged with how it was avoided -- not a synthetic
    /// "Miss"/"Block"/... ability the defender "used"
    fn record_avoided(&mut self, ts: Millis, src: &str, dst: &str, verb: &str, mitigation: Flags) {
        let enc = self
            .current_encounter_of(src)
            .or_else(|| self.current_encounter_of(dst));
        let a = self.sym(src);
        let t = self.sym(dst);
        self.clear_dead_if_acting(ts, a);
        let canonical = canonical_melee_ability(verb);
        // why: an avoided real special attack -- see record_damage's own
        // matching hook, and skilltracker.rs's own doc
        if src.eq_ignore_ascii_case("you") {
            crate::skilltracker::observe_skill_use(&mut self.skills, ts, canonical, false);
        }
        let ab = self.store.ability_id(canonical, tag::MELEE);
        let tier = self.current_tier(ts);
        let idx = self.store.push(
            ts,
            EventKind::Miss,
            a,
            t,
            ab,
            0,
            mitigation,
            enc.map(|e| e.0).unwrap_or(NO_ENCOUNTER),
            tier,
        );
        if let Some(id) = enc {
            self.store.extend_encounter(id, idx);
        }
    }

    fn record_death(&mut self, ts: Millis, victim: &str) {
        self.encounters.death(ts, victim);
        // why: resolves pending_xp; must run after death() (encounter
        // findable) and before drain_closed (before eviction)
        if let Some(p) = self.pending_xp.take() {
            if p.ts == ts {
                // why: same second as this death, consumed either way -- a
                // non-resolving match won't do better waiting for a later death
                if let Some(id) = self.encounter_id_for_victim(victim) {
                    self.store.enc[p.row as usize] = id.0;
                }
            } else {
                // why: not this death's to claim, put back for whatever death shares its timestamp
                self.pending_xp = Some(p);
            }
        }
        let sym = self.sym(victim);
        self.timeline.observed(ts, sym.0, State::Dead);
        self.drain_closed();
    }

    /// why: most recent encounter targeting victim, for pending_xp
    /// attribution -- deliberately not recent_encounter_for (that one's
    /// claim-tracking is for loot specifically; XP resolves at the exact
    /// moment a death closes, only one real answer, a plain reverse scan is enough)
    fn encounter_id_for_victim(&self, victim: &str) -> Option<EncounterId> {
        self.store
            .encounters
            .iter()
            .rev()
            .find(|e| self.store.name(e.target).eq_ignore_ascii_case(victim))
            .map(|e| e.id)
    }

    /// why: always self-directed, actor/target both "You"; `ability`
    /// reuses the interner for scope, not a dedicated column. `amount` is
    /// milli-percent, preserving 3 decimal digits in a u64.
    /// `enc` starts NO_ENCOUNTER, filled in later by record_death -- XP
    /// arrives before its own death line, nothing to search for yet.
    /// Confirmed real: fires for both kill XP and quest turn-in XP; only
    /// the first has a death to attach to, the second stays NO_ENCOUNTER forever, correctly.
    fn record_xp(&mut self, ts: Millis, scope: &str, pct: f64) {
        let scope = match scope.trim() {
            "party" => "party",
            "group" => "group",
            "raid" => "raid",
            _ => "solo",
        };
        let you = self.sym("You");
        let ab = self.store.ability_id(scope, 0);
        let milli_pct = (pct * 1000.0).round().clamp(0.0, u64::MAX as f64) as u64;
        let tier = self.current_tier(ts);
        let row = self.store.push(
            ts,
            EventKind::Xp,
            you,
            you,
            ab,
            milli_pct,
            0,
            NO_ENCOUNTER,
            tier,
        );
        self.pending_xp = Some(PendingXp { row, ts });
    }

    /// why: always the player, same self-directed shape as record_xp;
    /// a zero-parse is dropped not pushed -- a zero row would look like real data
    fn record_currency(&mut self, ts: Millis, source: &str, text: &str) {
        let copper = parse_currency_copper(text);
        if copper == 0 {
            return;
        }
        let you = self.sym("You");
        let ab = self.store.ability_id(source, 0);
        let tier = self.current_tier(ts);
        self.store.push(
            ts,
            EventKind::Currency,
            you,
            you,
            ab,
            copper,
            0,
            NO_ENCOUNTER,
            tier,
        );
    }

    /// why: best-effort EncounterId, not NO_ENCOUNTER -- the kill has
    /// almost always closed by loot time, no live fight to link to
    /// normally. recent_encounter_for matches kill order, not just
    /// recency. `monsters`' own aggregation doesn't depend on this at all
    /// (groups by target text); this is for "what did this pull drop" call sites.
    fn record_loot(&mut self, ts: Millis, item: &str, corpse: &str, qty: u64, sold: bool) {
        let mob = strip_corpse_suffix(corpse);
        let looter = self.sym("You");
        let target = self.sym(mob);
        let ab = self.store.ability_id(item, 0);
        let tier = self.current_tier(ts);
        let enc = self
            .recent_encounter_for(mob, ts)
            .map(|id| id.0)
            .unwrap_or(NO_ENCOUNTER);
        // why: flagged on the row itself, not left to a same-timestamp
        // Currency-row correlation a busy multi-item corpse could make ambiguous
        let flags = if sold { flag::LOOT_AUTO_SOLD } else { 0 };
        self.store.push(
            ts,
            EventKind::Loot,
            looter,
            target,
            ab,
            qty,
            flags,
            enc,
            tier,
        );
    }

    /// why: best-effort encounter for a loot line, matches kill order not
    /// just recency -- the naive "most recent same-named encounter"
    /// breaks once a third same-named mob dies before the first corpse is looted.
    ///
    /// Two-part rule:
    /// 1. `loot_cursor`: reuse the currently-claimed encounter if it's
    ///    still within LOOT_GRACE_MS of its own last activity (not the
    ///    gap since the last loot line) -- lets a slow manual loot window
    ///    still land right. An earlier version tracked its own separate
    ///    sticky gap and broke exactly this case once that gap lapsed.
    /// 2. Otherwise advance to the oldest unclaimed same-named encounter
    ///    within LOOT_GRACE_MS, marking it claimed.
    ///
    /// Known trade-off: two same-named mobs killed close together where
    /// the first's window stays open long can still have the second
    /// "win" the cursor and later get re-claimed onto the first -- rule 1
    /// only protects one slowly-resolved corpse, not several interleaved
    /// ones, the real reported case.
    ///
    /// Full scan of Store::encounters per call, not windowed -- runs
    /// once at ingest time, thousands of encounters is well under cost that mattered elsewhere.
    fn recent_encounter_for(&mut self, mob: &str, ts: Millis) -> Option<EncounterId> {
        let key = mob.to_ascii_lowercase();
        if let Some(&id) = self.loot_cursor.get(&key) {
            if let Some(e) = self.store.encounter(id) {
                let last_activity = e.end_ms.unwrap_or_else(|| {
                    self.store
                        .ts
                        .get(e.last as usize)
                        .copied()
                        .unwrap_or(e.start_ms)
                });
                if ts.saturating_sub(last_activity) <= crate::combat::LOOT_GRACE_MS {
                    return Some(id);
                }
            }
        }
        let id = {
            let store = &self.store;
            let claimed = &self.loot_claimed;
            store
                .encounters
                .iter()
                .filter(|e| {
                    store.name(e.target).eq_ignore_ascii_case(mob) && !claimed.contains(&e.id)
                })
                .filter(|e| {
                    let last_activity = e.end_ms.unwrap_or_else(|| {
                        store.ts.get(e.last as usize).copied().unwrap_or(e.start_ms)
                    });
                    e.start_ms <= ts
                        && ts.saturating_sub(last_activity) <= crate::combat::LOOT_GRACE_MS
                })
                .min_by_key(|e| e.start_ms)?
                .id
        };
        self.loot_claimed.insert(id);
        self.loot_cursor.insert(key, id);
        Some(id)
    }

    /// why: acting after Dead is itself proof of life -- the log rarely
    /// states a clean respawn line, especially for a corpse run, so
    /// recovery is inferred from the next action instead of waited for
    fn clear_dead_if_acting(&mut self, ts: Millis, actor: Sym) {
        if matches!(self.timeline.state_at(actor.0, ts), Some((State::Dead, _))) {
            self.timeline.inferred(ts, actor.0, State::Engaged);
        }
    }

    /// why: canonical first-observed casing, registers the entity if this is the first sighting
    fn resolve_name(&mut self, name: &str) -> String {
        self.encounters.entities.observe(name);
        self.encounters.entities.display_name(name).to_string()
    }

    /// why: interns via inferred pet ownership first, so a merged pet's
    /// rows all land on the owner's Sym; also case-folds so casing can't split one entity into two
    fn sym(&mut self, name: &str) -> Sym {
        let resolved = self.resolve_name(name);
        let effective = self.pet_owner.get(&resolved).cloned().unwrap_or(resolved);
        self.store.sym(&effective)
    }

    /// why: real bug, caught live -- a pet's own cast was feeding
    /// classdetect as if it were the owner's, on the strength of the
    /// merge `sym()` already does for DPS attribution. Wrong even when
    /// the merge itself is correct: a pet's ability kit doesn't match
    /// its owner's class the way a player's own cast does (a real
    /// Beastlord/Necromancer/Magician/Shaman pet has its own separate
    /// spell list) -- so a pet casting doesn't prove anything about
    /// which class the *owner* currently has active. Confirmed live:
    /// the specific incident that surfaced this was actually a 2nd,
    /// deeper bug (the merge itself was wrong -- an unrelated real
    /// player's spawn-buff cast collided with a real pet-summon sighting
    /// within the same PET_MATCH_WINDOW_MS and got merged in), but this
    /// check is right regardless of whether the merge is accurate.
    fn is_pet(&mut self, name: &str) -> bool {
        let resolved = self.resolve_name(name);
        self.pet_owner.contains_key(&resolved)
    }

    /// why: only from Cast, not damage/heal/miss -- a pet's first logged
    /// action is reliably its own spawn self-buff, and this scope was
    /// what was actually measured safe (see PET_MATCH_WINDOW_MS).
    /// Matches the closest-in-time pending summon, not "exactly one, else
    /// give up" -- real raid buff-up summons several pets within seconds
    /// of each other, and the game sometimes logs one summon twice at an
    /// identical timestamp, which alone broke "exactly one". Closest-in-time
    /// matched every case the stricter rule missed with no bad pairing in a spot check.
    fn note_actor(&mut self, ts: Millis, name: &str) {
        let resolved = self.resolve_name(name);
        if self.pet_owner.contains_key(&resolved) {
            return; // why: already resolved, nothing to check
        }
        if !self.seen_actors.insert(resolved.clone()) {
            return; // why: not their first time acting
        }
        // why: never a pet -- "You" and anyone already proven a player are
        // unambiguous and cost nothing to exclude; an unspoken real player
        // stays unprotected, the same known list_allies ceiling
        if resolved.eq_ignore_ascii_case("you")
            || self.encounters.entities.kind(&resolved) == Kind::Player
        {
            return;
        }
        self.pending_summons
            .retain(|(sts, _)| ts - *sts <= PET_MATCH_WINDOW_MS);
        let closest = self
            .pending_summons
            .iter()
            .enumerate()
            .min_by_key(|(_, (sts, _))| (ts - sts).abs())
            .map(|(i, _)| i);
        if let Some(i) = closest {
            let (_, owner) = self.pending_summons.remove(i);
            self.pet_owner.insert(resolved, owner);
        }
        // why: no pending summons at all, leave unresolved
    }

    /// why: registers owner as a pending candidate for whichever new entity acts next
    fn note_pet_summon(&mut self, ts: Millis, owner: &str) {
        let resolved = self.resolve_name(owner);
        self.pending_summons.push((ts, resolved));
    }

    /// why: opens a window during which an unmatched line gets checked against the flavor dictionary
    fn note_quickbuff(&mut self, ts: Millis, who: &str) {
        let resolved = self.resolve_name(who);
        self.pending_quickbuff.insert(resolved, ts);
    }

    /// why: live-tail's entry point for checking an unmatched line against
    /// the flavor dictionary; backfill does this itself during parallel
    /// classification instead (no Ingest access there). Four checks,
    /// cheapest/most specific first: (1) direct first-person hit --
    /// always a state ping, conditionally class evidence inside a
    /// still-open Quick Buff window; (2) third-person possessive; (3)
    /// third-person conjugated; (4) spell-identity dictionary (a
    /// different lookup from (1)-(3): names the exact spell, not just
    /// its classes, so it can also confirm a pending cast actually
    /// landed -- see `confirm_spell_effect`). Return value tells the
    /// caller whether the line is understood, just not by a rule.
    fn flavor_evidence_for(&mut self, ts: Millis, text: &str) -> bool {
        let classes = crate::flavordata::classes_for_flavor(text);
        if !classes.is_empty() {
            self.record_effect_ping(ts, "You", text);
            self.attribute_flavor_hit(ts, text, classes);
            return true;
        }
        if let Some((who, canonical)) = third_person_flavor(text) {
            self.record_effect_ping(ts, &who, &canonical);
            return true;
        }
        if let Some((who, canonical)) = verb_conjugated_flavor(text) {
            self.record_effect_ping(ts, &who, &canonical);
            return true;
        }
        if let Some(m) = crate::spelltext::match_spell_text(text) {
            self.confirm_spell_effect(ts, m);
            return true;
        }
        // why: fallback for text match_spell_text had to drop as
        // ambiguous -- still real, still worth a ping, just under the
        // line's own raw text since there's no one name to attach it to.
        if let Some(m) = crate::spelltext::match_effect_polarity(text) {
            self.record_effect_ping(ts, &m.target, text);
            return true;
        }
        false
    }

    /// why: shared by the live path (flavor_evidence_for) and backfill's
    /// sequential merge -- a landing confirms whatever pending cast of
    /// this exact spell "You" have open (a safe no-op if there isn't
    /// one, see `CastResolver::resolve`'s own doc); a wears-off is the
    /// opposite end of an already-landed buff, nothing to resolve.
    fn confirm_spell_effect(&mut self, ts: Millis, m: crate::spelltext::SpellTextMatch) {
        self.record_effect_ping(ts, &m.target, m.spell);
        if !m.is_wearsoff {
            let you = self.sym("You").0;
            let spell_sym = self.store.sym(m.spell).0;
            self.casts.confirm_landed(ts, you, spell_sym);
        }
    }

    /// why: 3s -- real slack around a spell's own `casting_time` (log
    /// timestamps are whole seconds; group effects can land a beat after
    /// the caster's own client sees it) before a cast no longer explains
    /// a landing/wears-off line
    const ATTRIBUTION_TOLERANCE_MS: Millis = 3_000;

    /// why: is `e`'s own expected landing moment (cast start + its real
    /// casting_time from the catalog, 0 if unknown) within tolerance of
    /// `ts` -- the real timing signal that turns "someone cast something
    /// vaguely recently" into "this specific cast is what explains this
    /// specific line landing at this specific moment"
    fn cast_explains_ts(e: &RecentCast, ts: Millis) -> bool {
        let cast_ms = crate::spelldata::spell_by_name(&e.spell)
            .and_then(|s| s.casting_time)
            .map(|t| (t * 1000.0) as Millis)
            .unwrap_or(0);
        (ts - (e.ts + cast_ms)).abs() <= Self::ATTRIBUTION_TOLERANCE_MS
    }

    /// why: best-effort "who cast this, with what spell" for a landing/
    /// wears-off/state line -- real signal, most confident tier first:
    /// (1) `text` is already a real spell name (confirm_spell_effect and
    /// ability.activated both feed record_effect_ping this way already
    /// resolved) -- skill is just that name; (2) a flavor sentence
    /// `spelltext::match_spell_text` resolves confidently catalog-wide --
    /// same as (1) once resolved; (3) spelltext.rs had to drop it as
    /// globally ambiguous (shared by several spells catalog-wide), but
    /// *locally* -- checked only against the spells actually cast nearby
    /// in this real log, via each candidate's own real `casting_time` --
    /// it can still be a confident, unique answer even though the global
    /// dictionary couldn't give one. Every tier requires exactly one real
    /// candidate; 0 or 2+ is an honest `None`, never a guess.
    ///
    /// Real gap, caught live: tier 3's own candidate check used to
    /// compare `text` against only msg_cast_on_you/msg_wears_off --
    /// never msg_cast_on_other, the ONE message shape that actually
    /// matters for a debuff landing on a TARGET (not on "You"). A whole
    /// spell line sharing its own landing flavor text catalog-wide
    /// (real: Tashan/Tashani/Tashania/Tashanian/Tashina/Wind of
    /// Tashani all share "Someone glances nervously about.") always got
    /// dropped by spelltext.rs's own global ambiguity check, and tier 3
    /// never got a chance to resolve it locally the way it already does
    /// for msg_cast_on_you -- "when a spell lands, the timer isnt going
    /// up, as if it landed". Checked via other_tail_of's own placeholder
    /// strip (same transform spelltext.rs's own dictionary build uses)
    /// plus a suffix match, since `text` here still carries the real
    /// target's own name where the catalog carries "Someone".
    fn attribute_effect(&self, ts: Millis, text: &str) -> (Option<String>, Option<String>) {
        let known_skill = if crate::spelldata::spell_by_name(text).is_some() {
            Some(text.to_string())
        } else {
            crate::spelltext::match_spell_text(text).map(|m| m.spell.to_string())
        };

        let candidates: Vec<&RecentCast> = self
            .recent_casts
            .entries
            .iter()
            .filter(|e| Self::cast_explains_ts(e, ts))
            .filter(|e| match &known_skill {
                Some(skill) => &e.spell == skill,
                None => crate::spelldata::spell_by_name(&e.spell).is_some_and(|sd| {
                    sd.msg_cast_on_you.as_deref() == Some(text)
                        || sd.msg_wears_off.as_deref() == Some(text)
                        || sd
                            .msg_cast_on_other
                            .as_deref()
                            .and_then(crate::spelltext::other_tail_of)
                            .is_some_and(|tail| text.ends_with(tail))
                }),
            })
            .collect();

        let casters: HashSet<u32> = candidates.iter().map(|e| e.caster).collect();
        let source = match casters.into_iter().collect::<Vec<_>>().as_slice() {
            [one] => Some(self.store.name(Sym(*one)).to_string()),
            _ => None,
        };
        let skill = known_skill.or_else(|| match candidates.as_slice() {
            [one] => Some(one.spell.clone()),
            _ => None,
        });
        (source, skill)
    }

    /// why: unconditional timestamped ping on target_name, resolved
    /// through pet ownership. Also the other half of
    /// attribute_flavor_hit's group-cast check -- every landing passes
    /// through here, disproving pending evidence in either time direction.
    fn record_effect_ping(&mut self, ts: Millis, target_name: &str, text: &str) {
        let resolved = self.resolve_name(target_name);
        let sym = self.sym(&resolved).0;
        let (source, skill) = self.attribute_effect(ts, text);
        self.effects
            .push(sym, ts, text.to_string(), source, skill, true);

        // why: cancel -- disproves a pending entry via a group cast (other
        // entity) or a pulsing buff (same entity again); ts != p.ts excludes self-cancel
        self.pending_quickbuff_evidence.retain(|p| {
            if p.text != text {
                return true;
            }
            let group_cast = p.who != sym && (ts - p.ts).abs() <= GROUP_CAST_WINDOW_MS;
            let pulsing = p.who == sym && ts != p.ts && (ts - p.ts).abs() <= PULSE_WINDOW_MS;
            !(group_cast || pulsing)
        });
        // why: commit -- pending entries past the cancellation window with nothing to disprove them
        let (still_pending, ready): (Vec<_>, Vec<_>) =
            std::mem::take(&mut self.pending_quickbuff_evidence)
                .into_iter()
                .partition(|p| ts - p.ts <= PULSE_WINDOW_MS);
        self.pending_quickbuff_evidence = still_pending;
        for p in ready {
            self.classes
                .observe_cast(p.who, self.zone.index_at(p.ts), p.classes);
        }

        self.recent_flavor_landings
            .retain(|(t, ..)| ts - *t <= GROUP_CAST_WINDOW_MS);
        self.recent_flavor_landings
            .push((ts, sym, text.to_string()));
    }

    /// why: tentatively attributes classes for whoever's Quick Buff
    /// window is open -- only when exactly one is open (two overlapping
    /// activators makes it ambiguous). Also guards against two other real
    /// false positives, both confirmed against the reference log: a
    /// group-wide buff landing on the activator during their own window
    /// (110 of 240 real activations hit this), and a maintained
    /// single-target buff pulsing on a regular cadence (confirmed ~6s
    /// cadence, 4,180 real occurrences vs 240 real activations). Doesn't
    /// commit immediately -- queues evidence, record_effect_ping cancels
    /// it if either disproof shows up after the fact.
    fn attribute_flavor_hit(&mut self, ts: Millis, text: &str, classes: &'static [String]) {
        self.pending_quickbuff
            .retain(|_, t0| ts - *t0 <= QUICKBUFF_WINDOW_MS);
        if self.pending_quickbuff.len() != 1 || classes.is_empty() {
            return;
        }
        let who = self.pending_quickbuff.keys().next().unwrap().clone();
        let sym = self.sym(&who).0;
        let group_cast_already = self
            .recent_flavor_landings
            .iter()
            .any(|(t, e, txt)| *e != sym && txt == text && (ts - *t).abs() <= GROUP_CAST_WINDOW_MS);
        let already_pulsing = self
            .effects
            .recent(sym, ts, PULSE_WINDOW_MS)
            .iter()
            .filter(|p| p.text == text)
            .count()
            > 1; // why: > 1 because text's own current landing is already in there
        if group_cast_already || already_pulsing {
            return;
        }
        self.pending_quickbuff_evidence
            .push(PendingQuickbuffEvidence {
                ts,
                who: sym,
                classes,
                text: text.to_string(),
            });
    }

    /// why: resolved through pet ownership, for callers walking
    /// entities_by_enc -- that list is raw, untouched by pet merging
    pub fn effective_name(&self, name: &str) -> String {
        let resolved = self.encounters.entities.display_name(name).to_string();
        self.pet_owner.get(&resolved).cloned().unwrap_or(resolved)
    }

    /// why: pets matched so far, surfaced in Overview so the inference is visible not silent
    pub fn pet_owner_count(&self) -> usize {
        self.pet_owner.len()
    }

    /// why: routes one damage edge through the graph, resolves to a
    /// store EncounterId, opening one the first time this component is
    /// seen. A merged-away component keeps its own store counterpart, so
    /// merge pushes a Closed record directly rather than leaving it open forever.
    fn link(&mut self, ts: Millis, actor: &str, target: &str) -> EncounterId {
        let enc_id = self.encounters.damage(ts, actor, target);

        // why: "You" checked alongside proven identity -- Kind::Player for
        // "You" only proves once they've spoken on a player channel.
        // Checks Allegiance::of(kind, state) as of this edge's own
        // timestamp, not raw Kind -- a currently-charmed ally fights for
        // the other side for as long as that lasts. Still not exhaustive:
        // an unspoken ally reads as a real mob, same known ceiling as list_allies.
        let actor_ally = self.is_ally(actor, ts);
        let target_ally = self.is_ally(target, ts);
        // why: None when both sides look like allies (self-inflicted
        // damage, ally-on-ally noise) or both look like mobs -- no opinion on which side is the mob
        let mob_side = match (actor_ally, target_ally) {
            (false, true) => Some(actor),
            (true, false) => Some(target),
            _ => None,
        };

        let store_id = if let Some(&id) = self.enc_map.get(&enc_id) {
            // why: retargets a stale anchor -- a boss's opening swing can
            // land on an unspoken groupmate first, before "You" hits the
            // real boss moments later (common, a raid tank eating hundreds
            // of hits). Never retargets away from an already-good anchor,
            // except when a real curated raid boss/miniboss joins an
            // already-open trash pull -- confirmed real gap: a live group
            // Lady Vox kill recorded entirely under "An icy terror"'s
            // encounter (she engaged mid-pull, anchor never moved off the
            // trash mob that opened it), invisible to the Raiding tab.
            if let Some(mob) = mob_side {
                let current_anchor = self
                    .store
                    .encounter(id)
                    .map(|e| self.store.name(e.target).to_string());
                let anchor_is_stale = current_anchor
                    .as_deref()
                    .is_some_and(|name| self.is_ally(name, ts));
                let boss_just_joined = crate::raiding::is_curated_raid_target(mob)
                    && !current_anchor
                        .as_deref()
                        .is_some_and(crate::raiding::is_curated_raid_target);
                if anchor_is_stale || boss_just_joined {
                    let sym = self.sym(mob);
                    self.store.retarget_encounter(id, sym);
                }
            }
            id
        } else {
            // why: first edge of a new fight, this guess is all there is;
            // falls back to target when ambiguous, correctable later
            let anchor = mob_side.unwrap_or(target);
            let target_sym = self.sym(anchor);
            let idx_hint = self.store.len() as u32;
            let zone_sym = self.current_zone(ts);
            let id = self
                .store
                .open_encounter(target_sym, ts, idx_hint, zone_sym);
            self.enc_map.insert(enc_id, id);
            id
        };
        if let Some(live) = self.encounters.live(enc_id) {
            self.entities_by_enc.insert(store_id, live.entities.clone());
        }
        store_id
    }

    /// why: ally as of ts, not forever -- a currently-charmed player/pet
    /// reads as enemy for as long as that lasts. Shared by link's new-fight and retarget paths.
    fn is_ally(&self, name: &str, ts: Millis) -> bool {
        let kind = if name.eq_ignore_ascii_case("you") {
            Kind::Player
        } else {
            self.encounters.entities.kind(name)
        };
        // why: read-only, never interns -- an ally check must never
        // itself create identity; no Sym yet defaults correctly to Engaged
        let canonical = self.encounters.entities.display_name(name);
        let state = self
            .store
            .names
            .get(canonical)
            .and_then(|sym| self.timeline.state_at(sym.0, ts))
            .map(|(s, _)| s)
            .unwrap_or(State::Engaged);
        !Allegiance::of(kind, state).is_enemy()
    }

    fn current_encounter_of(&self, name: &str) -> Option<EncounterId> {
        let enc_id = self.encounters.encounter_of(name)?;
        self.enc_map.get(&enc_id).copied()
    }

    /// why: drains newly-closed graph encounters into the store; Builder::closed only grows
    fn drain_closed(&mut self) {
        while self.closed_seen < self.encounters.closed.len() {
            // why: cloned not borrowed -- sym() below needs &mut self,
            // can't coexist with a borrow into encounters.closed
            let c = self.encounters.closed[self.closed_seen].clone();
            if let Some(&store_id) = self.enc_map.get(&c.id) {
                // why: c.slain mixes both sides -- a confirmed kill needs a real enemy name, not just any
                let confirmed_kill = c.slain.iter().any(|n| !self.is_ally(n, c.end_ms));
                let wiped = !confirmed_kill && c.slain.iter().any(|n| self.is_ally(n, c.end_ms));
                self.store
                    .close_encounter(store_id, c.end_ms, confirmed_kill, wiped);
                self.record_history(store_id, &c, confirmed_kill);
            }

            // why: alive-and-unaccounted-for left for an unreported reason
            // (blur, pacify, fleeing) -- marked Lost/Inferred not stuck Engaged. Players excluded.
            for name in &c.entities {
                if c.slain.iter().any(|s| s == name)
                    || self.encounters.entities.kind(name) == Kind::Player
                {
                    continue;
                }
                let sym = self.sym(name);
                if !matches!(
                    self.timeline.state_at(sym.0, c.end_ms),
                    Some((State::Dead, _))
                ) {
                    self.timeline.inferred(c.end_ms, sym.0, State::Lost);
                }
            }

            self.closed_seen += 1;
        }
    }

    /// why: builds one ParseRecord for a just-closed encounter, scoped to
    /// player's own damage only; no I/O, see crate::history for who persists these
    fn record_history(
        &mut self,
        store_id: EncounterId,
        c: &eqlp_session::Closed,
        confirmed_kill: bool,
    ) {
        let Some(target_sym) = self.store.encounter(store_id).map(|e| e.target) else {
            return;
        };
        let target = self.store.name(target_sym).to_string();
        let you = self.sym("You");
        let actual = by_ability(&self.store, &Filter::encounter(store_id).damage().by(you));
        if actual.is_empty() {
            // why: player dealt no damage in this fight, nothing to record
            return;
        }
        let zone = self.zone.at(c.start_ms).unwrap_or("Unknown").to_string();
        // why: scoped to same target + same tier, not every fight ever --
        // a nuke's expected damage depends on both. Self-diluted (this
        // encounter's own hits are in the baseline, no cheap way to
        // exclude). Skipped during backfill (!self.live) -- the baseline
        // query scans the whole store, O(store length) many times over
        // would be the same quadratic cost that made big-log replay crawl;
        // a backfilled record just carries no score, live closes still score normally.
        let score_ratio = self
            .live
            .then(|| {
                let tier = crate::zone::zone_tier(&zone).1;
                let baseline = by_ability(
                    &self.store,
                    &Filter::default()
                        .damage()
                        .by(you)
                        .target(target_sym)
                        .tier(tier),
                );
                score_parse(&baseline, &actual, &GearModifiers::default())
            })
            .and_then(|score| (!score.per_ability.is_empty()).then_some(score.ratio));
        let player_damage: u64 = actual.iter().map(|r| r.total).sum();
        let duration_ms = c.duration_ms().max(1);

        // why: confirmed classes as of exactly this point in the
        // sequential replay -- honest "as of this fight" answer, already
        // alphabetical so same-configuration fights group in by_loadout
        let zone_visit = self.zone.index_at(c.start_ms);
        let loadout: Vec<String> = self.classes.configuration_of_visit(you.0, zone_visit);

        self.pending_history.push(ParseRecord {
            target,
            zone,
            loadout,
            zone_visit,
            start_ms: c.start_ms,
            duration_ms,
            player_damage,
            player_dps: player_damage as f64 / (duration_ms as f64 / 1000.0),
            confirmed_kill,
            score_ratio,
        });
    }
}

/// why: one line's meaning, fully extracted to owned data, independent
/// of Match/line so it can cross a thread boundary. Produced by
/// extract_action, consumed by Ingest::apply.
enum Action {
    Damage {
        src: String,
        dst: String,
        ability: String,
        tags: Tags,
        amount: u64,
        flags: Flags,
    },
    /// why: dst may still be a reflexive pronoun, resolved in apply not here
    Heal {
        src: String,
        dst: String,
        ability: String,
        amount: u64,
    },
    /// why: verb is the attack-type so an avoided swing lands on the same
    /// row a landed one would; flags is the same pre-parsed trailing flag melee.hit carries
    Miss {
        src: String,
        dst: String,
        verb: String,
        flags: Flags,
    },
    /// why: same shape as Miss, kept distinct so accuracy can say "blocked"
    Block {
        src: String,
        dst: String,
        verb: String,
        flags: Flags,
    },
    /// why: same as Block, for a dodge
    Dodge {
        src: String,
        dst: String,
        verb: String,
        flags: Flags,
    },
    /// why: same as Block, for a parry
    Parry {
        src: String,
        dst: String,
        verb: String,
        flags: Flags,
    },
    Death {
        victim: String,
    },
    Zone {
        zone: String,
    },
    /// why: always the player, first-person only; the effective account level, not any one class's
    LevelUp {
        level: u8,
    },
    /// why: aa.gained is always rank 1 (never stated), aa.improved parses the trailing digit
    AaGained {
        name: String,
        rank: u8,
        cost: u32,
    },
    /// why: a "Beginning to..." line, proof of Possible-tier
    SpellBegan {
        name: String,
    },
    /// why: a "finished..." line, proof of Known-tier
    SpellFinished {
        name: String,
    },
    Cast {
        who: String,
        spell: String,
    },
    /// why: three real phrasings resolve here now -- "X resisted your Y!"
    /// (source hardcoded "You" at the call site), "You resist X's Y!",
    /// and third-party "X resisted Y's Z!" (neither side is You; still
    /// real, still worth resolving -- the cast tracker is multi-actor
    /// already, see `Action::Cast`'s own handling). `target` is who
    /// resisted -- only populated for the first phrasing (the other two
    /// don't name a fight the player is on the casting side of, nothing
    /// for the Skill Tracker's target-effects section to attribute).
    CastResisted {
        source: String,
        spell: String,
        target: Option<String>,
    },
    /// why: same shape as `state.charm_broken`'s own "worn off of
    /// <target>" rule, but for a pet's buff specifically -- no target to
    /// extract, always "Your pet"
    PetSpellWoreOff {
        spell: String,
    },
    /// why: source already resolved to a bare name, possessive stripped by the pattern
    CastInterrupted {
        source: String,
        spell: String,
    },
    /// why: same source shape as CastInterrupted
    CastFizzled {
        source: String,
        spell: String,
    },
    /// why: always the player's own cast, real class evidence; blocker
    /// names an already-active buff (stacking conflict, not a resist), None for no parenthetical
    CastBlocked {
        spell: String,
        target: String,
        blocker: Option<String>,
    },
    /// why: "Your X spell on Y has been overwritten" -- a REAL,
    /// unambiguous landing confirmation, always the player's own cast
    /// (packs/eql.toml's own note: "Self-only phrasing seen so far").
    /// Real gap, caught live: recognized as a known line shape for a
    /// while but never dispatched to anything at all -- a debuff
    /// re-applied onto a target that already had it (refreshing
    /// duration) never once names itself in the generic "You slow
    /// down."-style flavor text that pathway needs, so a spell like
    /// Shiftless Deeds could land, over and over, and target_effects'
    /// own panel would never see it -- "but it not showing the
    /// shiftless deeds slow". No attribution ambiguity here at all
    /// (unlike attribute_effect's own tier 3): spell and target are
    /// both named directly in the line.
    SpellOverwritten {
        spell: String,
        who: String,
    },
    /// why: a named condition landing on target, fed to Effects; text is
    /// a fixed label not scraped flavor text
    StateEffect {
        target: String,
        text: String,
    },
    /// why: /loc's own output, always the log owner. Does NOT share
    /// mapsdata.rs's axis order -- see Ingest::last_loc's real mapping.
    PlayerLoc {
        x: f64,
        y: f64,
        z: f64,
    },
    /// why: almost always third-person, so who is class evidence for
    /// that entity not necessarily "You"; also fed to Effects as a self-directed state fact
    /// `classdata` recognizes.
    AbilityActivated {
        who: String,
        ability: String,
    },
    /// "<Owner> summons a <flavour>." -- never names the pet itself, only
    /// the owner. See `Ingest::note_pet_summon`.
    PetSummon {
        owner: String,
    },
    /// "You assume a/an <stance> stance." -- self only, see `stancedata`
    /// for why this feeds `classes.observe_cast` the exact same way a
    /// class-restricted spell does.
    Stance {
        stance: String,
    },
    /// "You have become better at <skill>! (<level>)" -- self only, same
    /// evidence role as `Stance` but for a class-gated *skill* instead of
    /// a spell; see `skilldata` for which skills actually qualify (most
    /// don't -- a skill only counts if it's class-gated with no other
    /// route to it, like race).
    SkillUp {
        skill: String,
    },
    /// "You begin reciting the <invocation> invocation." -- self only,
    /// same evidence role as `Stance`; see `invocationdata`.
    Invocation {
        invocation: String,
    },
    PlayerProof {
        who: String,
    },
    /// why: Guild/Party/Raid/PM only -- says/shouts/auctions/OOC excluded,
    /// same real-player-channel filter `PlayerProof` already applies
    ChatMessage {
        who: String,
        channel: ChatChannel,
        text: String,
    },
    /// `ability.quickbuff`: "<Name> activates Quick Buff." -- opens a
    /// short window (`Ingest::note_quickbuff`) during which unmatched
    /// lines get checked against the buff-landing-message dictionary. See
    /// `crate::flavordata`'s module doc for why.
    QuickBuff {
        who: String,
    },
    Mez {
        who: String,
    },
    Charm {
        who: String,
    },
    /// why: charm wearing off, or the player's own mez ending
    Recovered {
        who: String,
    },
    /// why: self-only overlay signals -- see effects.rs's own doc. No
    /// payload: which literal variant fired (regular/undead/animal invis,
    /// "moved" vs "stop hiding", ...) doesn't matter downstream, only the
    /// semantic outcome does.
    InvisFading,
    InvisLanded,
    InvisEnded,
    HideSuccess,
    HideFailure,
    HideEnded,
    SneakSuccess,
    SneakFailure,
    SneakEnded,
    /// why: always the player, no third-person loot line exists. corpse
    /// keeps its raw suffix (stripped in record_loot, not here); sold_for
    /// present only for an auto-sell, raw and unparsed
    Loot {
        item: String,
        corpse: String,
        qty: u64,
        sold_for: Option<String>,
    },
    /// why: always the player; scope is the raw capture, empty for solo,
    /// normalized in record_xp not here
    Xp {
        scope: String,
        pct: f64,
    },
    /// why: from money.corpse or money.vendor_sell; loot.self.direct's
    /// auto-sell case goes through Loot's sold_for instead -- one line, two real facts
    Currency {
        source: String,
        text: String,
    },
    /// why: no fields, the line carries only the fact + timestamp, both apply already has
    AfkOn,
    AfkOff,
    /// why: client's confirmation an /outputfile command finished; only records the filename
    OutputfileComplete {
        file: String,
    },
    /// why: proc.item's Exaltation case, proof the Proc socket is genuinely live
    ExaltationProc {
        item: String,
    },
}

/// why: classifies what one matched line means without mutating
/// anything -- a pure function, runs on a backfill worker thread or inline live
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
    let f64_field = |name: &str| -> Option<f64> {
        match field::field(engine, m, line, name) {
            field::Value::F64(n) => Some(n),
            _ => None,
        }
    };

    match rule_id {
        "melee.hit" => {
            let (src, dst, amount) = (
                str_field("source")?,
                str_field("target")?,
                u64_field("amount")?,
            );
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
            let (src, dst, amount, spell) = (
                str_field("source")?,
                str_field("target")?,
                u64_field("amount")?,
                str_field("spell")?,
            );
            let flags = str_field("flag").map(|s| flag::parse(&s)).unwrap_or(0);
            Some(Action::Damage {
                src,
                dst,
                ability: spell,
                tags: tag::SPELL,
                amount,
                flags,
            })
        }
        "dot.damage" => {
            let (src, dst, amount, spell) = (
                str_field("source")?,
                str_field("target")?,
                u64_field("amount")?,
                str_field("spell")?,
            );
            let flags = str_field("flag").map(|s| flag::parse(&s)).unwrap_or(0);
            Some(Action::Damage {
                src,
                dst,
                ability: spell,
                tags: tag::SPELL | tag::DOT,
                amount,
                flags,
            })
        }
        "dot.damage_uncredited" => {
            // why: no caster named, attributed to a placeholder rather
            // than dropped so damage still counts against the target's total
            let (dst, amount, spell) = (
                str_field("target")?,
                u64_field("amount")?,
                str_field("spell")?,
            );
            Some(Action::Damage {
                src: "unknown".to_string(),
                dst,
                ability: spell,
                tags: tag::SPELL | tag::DOT,
                amount,
                flags: 0,
            })
        }
        "dot.damage_from_you" => {
            // why: caster named via possessive "your" in the line itself, always "You"
            let (dst, amount, spell) = (
                str_field("target")?,
                u64_field("amount")?,
                str_field("spell")?,
            );
            let flags = str_field("flag").map(|s| flag::parse(&s)).unwrap_or(0);
            Some(Action::Damage {
                src: "You".to_string(),
                dst,
                ability: spell,
                tags: tag::SPELL | tag::DOT,
                amount,
                flags,
            })
        }
        "ds.damage" => {
            // why: source names the effect not the entity ("X's flames") --
            // split so the wearer is the actor, not a separate entity
            let (raw_src, dst, amount) = (
                str_field("source")?,
                str_field("target")?,
                u64_field("amount")?,
            );
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
            let (src, dst, amount, spell) = (
                str_field("source")?,
                str_field("target")?,
                u64_field("amount")?,
                str_field("spell")?,
            );
            Some(Action::Heal {
                src,
                dst,
                ability: spell,
                amount,
            })
        }
        "heal.plain" => {
            let (src, dst, amount) = (
                str_field("source")?,
                str_field("target")?,
                u64_field("amount")?,
            );
            Some(Action::Heal {
                src,
                dst,
                ability: "Heal".to_string(),
                amount,
            })
        }
        "melee.miss" => {
            let (src, dst, verb) = (
                str_field("source")?,
                str_field("target")?,
                str_field("verb")?,
            );
            let flags = str_field("flag").map(|s| flag::parse(&s)).unwrap_or(0);
            Some(Action::Miss {
                src,
                dst,
                verb,
                flags,
            })
        }
        "melee.blocked" => {
            let (src, dst, verb) = (
                str_field("source")?,
                str_field("target")?,
                str_field("verb")?,
            );
            let flags = str_field("flag").map(|s| flag::parse(&s)).unwrap_or(0);
            Some(Action::Block {
                src,
                dst,
                verb,
                flags,
            })
        }
        "melee.dodged" => {
            let (src, dst, verb) = (
                str_field("source")?,
                str_field("target")?,
                str_field("verb")?,
            );
            let flags = str_field("flag").map(|s| flag::parse(&s)).unwrap_or(0);
            Some(Action::Dodge {
                src,
                dst,
                verb,
                flags,
            })
        }
        "melee.parried" => {
            let (src, dst, verb) = (
                str_field("source")?,
                str_field("target")?,
                str_field("verb")?,
            );
            let flags = str_field("flag").map(|s| flag::parse(&s)).unwrap_or(0);
            Some(Action::Parry {
                src,
                dst,
                verb,
                flags,
            })
        }
        "cast.begin" | "sing.begin" => {
            let who = str_field("source")?;
            let spell = str_field("spell").or_else(|| str_field("song"))?;
            Some(Action::Cast { who, spell })
        }
        "spell.resisted" => Some(Action::CastResisted {
            source: "You".to_string(),
            spell: str_field("spell")?,
            target: str_field("who"),
        }),
        "spell.you_resisted" | "spell.resisted_by" => Some(Action::CastResisted {
            source: str_field("caster")?,
            spell: str_field("spell")?,
            target: None,
        }),
        "cast.blocked" => Some(Action::CastBlocked {
            spell: str_field("spell")?,
            target: str_field("target")?,
            blocker: str_field("blocker"),
        }),
        "cast.blocked_self" => Some(Action::CastBlocked {
            spell: str_field("spell")?,
            target: "You".to_string(),
            blocker: str_field("blocker"),
        }),
        "state.spell_overwritten" => Some(Action::SpellOverwritten {
            spell: str_field("spell")?,
            who: str_field("who")?,
        }),
        "spell.pet_wore_off" => Some(Action::PetSpellWoreOff {
            spell: str_field("spell")?,
        }),
        "state.you_poisoned" => Some(Action::StateEffect {
            target: "You".to_string(),
            text: "Poisoned".to_string(),
        }),
        "state.poisoned" => Some(Action::StateEffect {
            target: str_field("who")?,
            text: "Poisoned".to_string(),
        }),
        "state.you_diseased" => Some(Action::StateEffect {
            target: "You".to_string(),
            text: "Diseased".to_string(),
        }),
        "state.diseased" => Some(Action::StateEffect {
            target: str_field("who")?,
            text: "Diseased".to_string(),
        }),
        "ability.activated" => Some(Action::AbilityActivated {
            who: str_field("who")?,
            ability: str_field("ability")?,
        }),
        "state.location" => Some(Action::PlayerLoc {
            x: f64_field("x")?,
            y: f64_field("y")?,
            z: f64_field("z")?,
        }),
        "cast.interrupted" => Some(Action::CastInterrupted {
            // why: source is absent only when the "Your" branch matched -- exact, not a guess
            source: str_field("source").unwrap_or_else(|| "You".to_string()),
            spell: str_field("spell")?,
        }),
        "cast.fizzled" => Some(Action::CastFizzled {
            source: str_field("source").unwrap_or_else(|| "You".to_string()),
            spell: str_field("spell")?,
        }),
        "death.you_slew" | "death.other" | "death.plain" => Some(Action::Death {
            victim: str_field("victim")?,
        }),
        "death.you_died" => {
            // why: synthesized, not read -- fold_key matches whatever casing "you" was seen under
            Some(Action::Death {
                victim: "You".to_string(),
            })
        }
        "zone.enter" => Some(Action::Zone {
            zone: str_field("zone")?,
        }),
        "level.up" => Some(Action::LevelUp {
            level: u64_field("level")?.min(u8::MAX as u64) as u8,
        }),
        "aa.gained" => Some(Action::AaGained {
            name: str_field("name")?,
            rank: 1,
            cost: u64_field("cost")?.min(u32::MAX as u64) as u32,
        }),
        "aa.improved" => Some(Action::AaGained {
            name: str_field("name")?,
            rank: u64_field("rank")?.min(u8::MAX as u64) as u8,
            cost: u64_field("cost")?.min(u32::MAX as u64) as u32,
        }),
        "spell.memorize_start" | "spell.scribe_start" => Some(Action::SpellBegan {
            name: str_field("spell")?,
        }),
        "spell.memorize_done" | "spell.scribe_done" => Some(Action::SpellFinished {
            name: str_field("spell")?,
        }),
        "pet.summoned" => Some(Action::PetSummon {
            owner: str_field("who")?,
        }),
        "state.stance" => Some(Action::Stance {
            stance: str_field("stance")?,
        }),
        "skill.up" => Some(Action::SkillUp {
            skill: str_field("skill")?,
        }),
        "state.invocation" => Some(Action::Invocation {
            invocation: str_field("invocation")?,
        }),
        "state.mesmerized" => Some(Action::Mez {
            who: str_field("who")?,
        }),
        "state.charmed" => Some(Action::Charm {
            who: str_field("who")?,
        }),
        "state.charm_broken" | "state.you_mesmerized" => Some(Action::Recovered {
            who: str_field("who").unwrap_or_else(|| "You".to_string()),
        }),
        "invis.fading" => Some(Action::InvisFading),
        "invis.landed.vanish" | "invis.landed.tingle" | "invis.landed.fade" => {
            Some(Action::InvisLanded)
        }
        "invis.ended.appear" | "invis.ended.tingle" | "invis.ended.fade" => {
            Some(Action::InvisEnded)
        }
        "hide.success" => Some(Action::HideSuccess),
        "hide.failure" => Some(Action::HideFailure),
        "hide.broken" | "hide.stopped" => Some(Action::HideEnded),
        "sneak.success" => Some(Action::SneakSuccess),
        "sneak.failure" => Some(Action::SneakFailure),
        "sneak.broken" => Some(Action::SneakEnded),
        // why: two client line forms for the same fact (bracketed vs
        // direct, varying trailing clause); both produce the identical Action::Loot
        "loot.self" | "loot.self.direct" => Some(Action::Loot {
            item: str_field("item")?,
            corpse: str_field("corpse")?,
            qty: u64_field("qty").unwrap_or(1),
            sold_for: str_field("sold_for"),
        }),
        "xp.gain" => Some(Action::Xp {
            scope: str_field("scope").unwrap_or_default(),
            pct: f64_field("pct")?,
        }),
        "money.corpse" => Some(Action::Currency {
            source: "corpse".to_string(),
            text: str_field("amount")?,
        }),
        "money.vendor_sell" => Some(Action::Currency {
            source: "vendor".to_string(),
            text: str_field("amount")?,
        }),
        "afk.on" => Some(Action::AfkOn),
        "afk.off" => Some(Action::AfkOff),
        "outputfile.complete" => Some(Action::OutputfileComplete {
            file: str_field("file")?,
        }),
        // why: proc.item is generic, only the Exaltation-labeled case is
        // this app's signal for a live Proc socket; every other effect value left unrecorded
        "proc.item" => {
            let (item, effect) = (str_field("item")?, str_field("effect")?);
            effect
                .eq_ignore_ascii_case("Exaltation")
                .then_some(Action::ExaltationProc { item })
        }
        "ability.quickbuff" => Some(Action::QuickBuff {
            who: str_field("who")?,
        }),
        "chat.channel" => Some(Action::PlayerProof {
            who: str_field("who")?,
        }),
        "chat.directed" => {
            // why: only provably player-to-player channels -- says/shouts/auctions excluded, NPCs use says too
            let (who, chan) = (str_field("who")?, str_field("chan")?);
            let channel = match chan.as_str() {
                "tells you" => Some(ChatChannel::Pm { with: who.clone() }),
                "tells the guild" | "tell the guild" => Some(ChatChannel::Guild),
                "tells the group" | "tell your party" | "tell the group" => {
                    Some(ChatChannel::Party)
                }
                "tells the raid" | "tell your raid" => Some(ChatChannel::Raid),
                _ => None,
            };
            match channel {
                // why: still proves who's a real player even when the
                // channel itself isn't one Social tracks (kept for
                // parity with the old player_only behavior)
                None => None,
                Some(channel) => {
                    let text = str_field("text")?;
                    Some(Action::ChatMessage { who, channel, text })
                }
            }
        }
        "chat.tell_sent" => Some(Action::ChatMessage {
            who: "You".to_string(),
            channel: ChatChannel::Pm {
                with: str_field("who")?,
            },
            text: str_field("text")?,
        }),
        _ => None,
    }
}

/// why: verb -> canonical ability name; pack regex alternates 3rd-person and base form, same ability
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

/// why: resolves a written reflexive pronoun back to the caster's real name
fn resolve_reflexive(target: &str, source: &str) -> String {
    match target {
        "himself" | "herself" | "itself" | "yourself" => source.to_string(),
        other => other.to_string(),
    }
}

/// why: names ending in what looks like a rank numeral that's actually
/// part of the spell's own identity (Yaulp/Yaulp II/Yaulp III are 3
/// different spells) -- confirmed via cross-reference with the real
/// spell scrape. A snapshot, regenerate after a fresh scrape.
const PROTECTED_SPELL_NAMES: &[&str] = &[
    "Burnout II",
    "Burnout III",
    "Clarity II",
    "Monster Summoning I",
    "Rune I",
    "Rune II",
    "Rune III",
    "Yaulp II",
    "Yaulp III",
];

/// why: real EQ convention -- a player character's own name is always
/// exactly one capitalized word, never lowercase-initial the way a
/// generic mob name always is ("a haunted chest", "an elemental
/// warrior", "a kobold watcher"). Used to guard promote_party_member's
/// own shared-target-damage heuristic -- see its own doc for the real
/// bug this closes. Deliberately loose (any uppercase-initial name
/// passes, including unique/named mobs like "Lord Nagafen") -- the
/// point isn't a perfect player detector, just ruling out the
/// overwhelming, confirmed-real false-positive shape outright.
fn plausible_player_name(name: &str) -> bool {
    name.chars().next().is_some_and(|c| c.is_uppercase())
}

/// why: strips a trailing rank numeral so a ranked cast name compares
/// against an unranked damage/heal line; checks PROTECTED_SPELL_NAMES first
fn base_spell_name(name: &str) -> &str {
    if PROTECTED_SPELL_NAMES.contains(&name) {
        return name;
    }
    match name.rsplit_once(' ') {
        Some((base, tail)) if is_roman_numeral(tail) => base,
        _ => name,
    }
}

/// why: 2nd real bug in the same family, spotted by the user -- "Illusion:
/// Dark Elf" (treated as rock-solid Enchanter-exclusive evidence, dozens
/// of real casts) is also a click effect on 2 real items (`Guise of the
/// Deceiver`, `Mask of Deception`, per `spells.json`'s own
/// `items_with_effect`), so a cast of it proves nothing about the
/// caster's own class -- only that they're holding the item. The log
/// line is identical either way ("You begin casting X"), so there's no
/// way to tell a real class cast from an item click after the fact; per
/// the same logic as a group teleport, unsure evidence is no evidence.
/// 669 of 1928 real spells in the catalog have at least one item source
/// -- broad enough that this needed its own cached lookup, not a
/// per-call linear scan over the whole catalog (`spell_by_name`'s own
/// doc: "not a hot path" -- this one is, called on every real cast line
/// in a multi-million-line backfill).
fn has_item_click_source(base_spell: &str) -> bool {
    static ITEM_SOURCED: OnceLock<std::collections::HashSet<&'static str>> = OnceLock::new();
    ITEM_SOURCED
        .get_or_init(|| {
            crate::spelldata::spells()
                .iter()
                .filter(|s| !s.items_with_effect.is_empty())
                .map(|s| s.name.as_str())
                .collect()
        })
        .contains(base_spell)
}

/// why: real bug, caught live against a real 2nd player's log -- a
/// group-shaped teleport ("Ring of Butcherblock", Druid-only per its own
/// class data) showed up cast inside a visit already rock-solid confirmed
/// as Cleric/Paladin/Shaman (dozens of each class's own exclusive spells),
/// mathematically impossible under the fixed-CLASS_COUNT rule if the
/// caster genuinely needed Druid active to cast it. Group teleports in
/// this game can be triggered as a party ritual, not gated to the
/// classes usually shown in `spell_classes.json` -- so a cast alone
/// doesn't prove the caster currently has that class active, only that
/// *someone* in the group does (or did, to have learned it). Landing
/// coordinates (this file's own `teleportdata::landing_for`) are exact,
/// hand-verified data for the map feature; reused here as the same
/// "is this a real teleport" signal rather than re-deriving one.
/// A spell obtainable from an item's click effect (see
/// `has_item_click_source`'s own doc) gets the same treatment.
fn class_evidence_for(base_spell: &str) -> &'static [String] {
    if teleportdata::landing_for(base_spell).is_some() || has_item_click_source(base_spell) {
        return &[];
    }
    crate::classdata::classes_for(base_spell)
}

fn is_roman_numeral(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| matches!(b, b'I' | b'V' | b'X' | b'L' | b'C' | b'D' | b'M'))
}

/// why: splits "Ice Comet X" -> ("Ice Comet", Some(10)). Two unrelated
/// roman-numeral phenomena: a numeral baked into the spell's own
/// identity (Monster Summoning II/III, no bare page exists), and a live
/// per-character rank appended only in log text, never the wiki title.
/// Disambiguated by checking the catalog directly: full name real -> no
/// rank; only base-after-stripping real -> observed rank.
pub(crate) fn split_cast_rank(name: &str) -> (&str, Option<u8>) {
    if crate::spelldata::spell_by_name(name).is_some() {
        return (name, None);
    }
    match name.rsplit_once(' ') {
        Some((base, tail))
            if is_roman_numeral(tail) && crate::spelldata::spell_by_name(base).is_some() =>
        {
            (base, roman_to_u8(tail))
        }
        _ => (name, None),
    }
}

/// why: subtractive-notation roman numeral -> integer, clamped to u8;
/// None for charset-valid but nonsensical ordering, not a wrong number
fn roman_to_u8(s: &str) -> Option<u8> {
    fn value(b: u8) -> u32 {
        match b {
            b'I' => 1,
            b'V' => 5,
            b'X' => 10,
            b'L' => 50,
            b'C' => 100,
            b'D' => 500,
            b'M' => 1000,
            _ => 0,
        }
    }
    let bytes = s.as_bytes();
    let mut total: u32 = 0;
    let mut i = 0;
    while i < bytes.len() {
        let cur = value(bytes[i]);
        let next = if i + 1 < bytes.len() {
            value(bytes[i + 1])
        } else {
            0
        };
        if cur < next {
            total += next - cur;
            i += 2;
        } else {
            total += cur;
            i += 1;
        }
    }
    u8::try_from(total).ok()
}

#[cfg(test)]
mod spell_rank_tests {
    use super::*;

    #[test]
    fn a_live_cast_rank_is_split_off_a_real_base_spell() {
        assert_eq!(split_cast_rank("Ice Comet X"), ("Ice Comet", Some(10)));
        assert_eq!(
            split_cast_rank("Garrison's Mighty Mana Shock IX"),
            ("Garrison's Mighty Mana Shock", Some(9))
        );
    }

    #[test]
    fn a_spell_line_variant_is_never_treated_as_a_rank() {
        // why: the exact bug reported -- "Monster Summoning" alone isn't
        // a real spell, only "Monster Summoning II"/"III" are, each its
        // own catalog entry; same for the Yaulp line.
        assert_eq!(
            split_cast_rank("Monster Summoning II"),
            ("Monster Summoning II", None)
        );
        assert_eq!(
            split_cast_rank("Monster Summoning III"),
            ("Monster Summoning III", None)
        );
        assert_eq!(split_cast_rank("Yaulp II"), ("Yaulp II", None));
        assert_eq!(split_cast_rank("Yaulp III"), ("Yaulp III", None));
    }

    #[test]
    fn an_unranked_cast_of_a_plain_spell_has_no_rank() {
        assert_eq!(split_cast_rank("Ice Comet"), ("Ice Comet", None));
    }

    #[test]
    fn an_unrecognized_name_never_fabricates_a_rank() {
        // why: charset-valid roman numeral, but the stripped base isn't a
        // real catalog spell either -- must not guess.
        assert_eq!(
            split_cast_rank("Some Made Up Ability X"),
            ("Some Made Up Ability X", None)
        );
    }

    #[test]
    fn roman_numerals_convert_correctly() {
        assert_eq!(roman_to_u8("I"), Some(1));
        assert_eq!(roman_to_u8("IV"), Some(4));
        assert_eq!(roman_to_u8("IX"), Some(9));
        assert_eq!(roman_to_u8("X"), Some(10));
    }

    #[test]
    fn spell_ranks_only_ever_keeps_the_highest_observed() {
        let mut r = SpellRanks::default();
        r.observe(0, "Ice Comet", 4);
        r.observe(1000, "Ice Comet", 9);
        r.observe(2000, "Ice Comet", 6); // why: a lower re-observation must not regress it
        assert_eq!(r.rank_of("Ice Comet"), Some(9));
        assert_eq!(r.rank_of("Never Cast"), None);
    }
}

#[cfg(test)]
mod party_promotion_tests {
    use super::*;
    use crate::parser::build_engine;

    fn run(lines: &[&str]) -> Ingest {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let bytes: Vec<&[u8]> = lines.iter().map(|l| l.as_bytes()).collect();
        backfill_lines(&mut ing, &engine, &bytes, 1);
        ing
    }

    #[test]
    fn plausible_player_name_rejects_generic_mob_shapes() {
        assert!(plausible_player_name("Kaeus"));
        assert!(plausible_player_name("Lord Nagafen")); // why: unique/named mobs still pass -- see its own doc
        assert!(!plausible_player_name("a haunted chest"));
        assert!(!plausible_player_name("an elemental warrior"));
        assert!(!plausible_player_name(""));
    }

    /// why: real bug, caught live -- "a haunted chest, only thing in
    /// combat... wasnt parsing to the ui for it, but it was parsing
    /// fine to dps meter". A mob dealing damage to an anchor "You" had
    /// already confirmed (here: two mobs cross-damaging each other)
    /// used to promote that mob to permanent Kind::Player via the same
    /// shared-target heuristic that's supposed to catch silent real
    /// party members -- poisoning every later encounter with that same
    /// name for the rest of the session. Spencer's own framing: real
    /// players stay a consistent classification, a mob is only ever a
    /// TEMPORARY ally (charm).
    #[test]
    fn a_mob_cross_damaging_a_confirmed_anchor_is_never_promoted_to_player() {
        let ing = run(&[
            "[Tue Jul 28 15:01:00 2026] You hit a bat for 5 points of damage.",
            "[Tue Jul 28 15:01:02 2026] a rat hit a bat for 3 points of damage.",
        ]);
        assert_eq!(ing.encounters.entities.kind("a rat"), Kind::Unproven);
    }

    /// why: the same heuristic must still work for its own real,
    /// intended purpose -- a genuine, silently-present party member
    /// (never spoken on a player channel) still gets recognized as one
    /// through the exact same shared-target-damage evidence
    #[test]
    fn a_real_silent_party_member_still_gets_promoted() {
        let ing = run(&[
            "[Tue Jul 28 15:01:00 2026] You hit a bat for 5 points of damage.",
            "[Tue Jul 28 15:01:02 2026] Groupmate hit a bat for 3 points of damage.",
        ]);
        assert_eq!(ing.encounters.entities.kind("Groupmate"), Kind::Player);
    }
}

/// why: splits into wearer + flavour word; falls back to the whole string as wearer, no panic
fn split_damage_shield_source(raw: &str) -> (String, String) {
    if let Some(flavour) = raw.strip_prefix("YOUR ") {
        return ("You".to_string(), flavour.to_string());
    }
    if let Some(pos) = raw.rfind("'s ") {
        let (wearer, rest) = raw.split_at(pos);
        let flavour = &rest[3..]; // why: skip "'s "
        if !wearer.is_empty() && !flavour.is_empty() {
            return (wearer.to_string(), flavour.to_string());
        }
    }
    (raw.to_string(), "Damage Shield".to_string())
}

/// why: corpse capture is always "<mob>'s corpse", confirmed real 546/546; strips to the mob's display name
fn strip_corpse_suffix(corpse: &str) -> &str {
    corpse.strip_suffix("'s corpse").unwrap_or(corpse)
}

/// why: classic EQ conversion (1p=10g=100s=1000c), nothing suggests EQL changed it
const CURRENCY_DENOMINATIONS: &[(&str, u64)] = &[
    ("platinum", 1000),
    ("gold", 100),
    ("silver", 10),
    ("copper", 1),
];

/// why: not a whole-clause regex -- the server phrases the same
/// 4-denomination list two different ways (comma-joined vs space-joined)
/// depending on the line, so this walks looking for pairs directly, order- and
/// separator-agnostic, rather than trying to encode every real variant
/// into one pattern. Any subset of the four may be present, in any
/// combination; an amount with none recognised (a parse this doesn't
/// handle, or genuinely no currency) reads as `0`, same as an empty
/// string -- see `record_currency`'s doc for why that's dropped rather
/// than stored.
fn parse_currency_copper(text: &str) -> u64 {
    let bytes = text.as_bytes();
    let mut total = 0u64;
    let mut i = 0usize;
    while i < bytes.len() {
        if !bytes[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        let Ok(n) = text[start..i].parse::<u64>() else {
            continue;
        };
        let mut j = i;
        while j < bytes.len() && bytes[j] == b' ' {
            j += 1;
        }
        let word_start = j;
        while j < bytes.len() && bytes[j].is_ascii_alphabetic() {
            j += 1;
        }
        if let Some(&(_, mult)) = CURRENCY_DENOMINATIONS
            .iter()
            .find(|&&(name, _)| name.eq_ignore_ascii_case(&text[word_start..j]))
        {
            total = total.saturating_add(n.saturating_mul(mult));
        }
        i = j;
    }
    total
}

// ---------------------------------------------------------------- parallel backfill

/// why: one classified line ahead of the sequential merge; the two flavor
/// variants mirror flavor_evidence_for's checks, run here since both are stateless
enum Classified {
    Action(Action),
    /// why: known first-person "You" landing message; classes feed Quick
    /// Buff attribution, text feeds the unconditional state ping
    SelfFlavorHit {
        classes: &'static [String],
        text: String,
    },
    /// why: known landing message about `who`, not "You"; never class evidence
    ThirdPersonFlavorHit {
        who: String,
        text: String,
    },
    /// why: spell-identity dictionary hit -- a different lookup from the
    /// two above, carries the spell itself so the sequential merge can
    /// also confirm a pending cast landed
    SpellEffectHit {
        spell: &'static str,
        target: String,
        is_wearsoff: bool,
    },
    /// why: match_spell_text's own fallback for text it had to drop as
    /// ambiguous -- polarity only (no name to confirm a cast against),
    /// same ping mechanism as the two FlavorHit variants above
    EffectPolarityHit {
        who: String,
        text: String,
    },
}

/// why: one chunk's classification, replayed sequentially; keeps every
/// matched timestamp even with no Classified so the log clock still advances in order
struct ChunkResult {
    counts: LineCounts,
    matched: Vec<(Millis, Option<Classified>)>,
    /// why: this chunk's local shape accumulation, folded into Ingest's
    /// map after merge; deliberately uncapped here (transient + already
    /// bounded by chunk size) -- a local cap under-/over-counted vs
    /// `eqlp coverage` when parallel chunks each dropped overflow blind to the global map
    unmatched_shapes: HashMap<Vec<u8>, ShapeStat>,
}

/// why: the expensive, embarrassingly-parallel classification step; no
/// Ingest access, safe to run on another thread
fn classify_chunk(engine: &Engine, lines: &[&[u8]]) -> ChunkResult {
    let mut matcher = engine.matcher();
    let mut counts = LineCounts::default();
    let mut matched = Vec::with_capacity(lines.len());
    let mut shaper = Shaper::new();
    let mut shape_scratch = Vec::new();
    let mut unmatched_shapes: HashMap<Vec<u8>, ShapeStat> = HashMap::new();
    for &line in lines {
        counts.total += 1;
        match matcher.classify(line) {
            Outcome::Matched(m) => {
                counts.matched += 1;
                let rule = engine.rule(m.rule);
                *counts.by_kind.entry(rule.kind.clone()).or_insert(0) += 1;
                let ts_ms = m.ts.secs() * 1000;
                let action = extract_action(engine, rule.id.as_str(), &m, line);
                matched.push((ts_ms, action.map(Classified::Action)));
            }
            Outcome::Unmatched { ts, body } => {
                counts.unmatched += 1;
                let text_bytes = body.slice(line);
                let text = String::from_utf8_lossy(text_bytes);
                let classes = crate::flavordata::classes_for_flavor(&text);
                let recognized = if !classes.is_empty() {
                    let ts_ms = ts.secs() * 1000;
                    matched.push((
                        ts_ms,
                        Some(Classified::SelfFlavorHit {
                            classes,
                            text: text.to_string(),
                        }),
                    ));
                    true
                } else if let Some((who, canonical)) = third_person_flavor(&text) {
                    let ts_ms = ts.secs() * 1000;
                    matched.push((
                        ts_ms,
                        Some(Classified::ThirdPersonFlavorHit {
                            who,
                            text: canonical,
                        }),
                    ));
                    true
                } else if let Some((who, canonical)) = verb_conjugated_flavor(&text) {
                    let ts_ms = ts.secs() * 1000;
                    matched.push((
                        ts_ms,
                        Some(Classified::ThirdPersonFlavorHit {
                            who,
                            text: canonical,
                        }),
                    ));
                    true
                } else if let Some(m) = crate::spelltext::match_spell_text(&text) {
                    let ts_ms = ts.secs() * 1000;
                    matched.push((
                        ts_ms,
                        Some(Classified::SpellEffectHit {
                            spell: m.spell,
                            target: m.target,
                            is_wearsoff: m.is_wearsoff,
                        }),
                    ));
                    true
                } else if let Some(m) = crate::spelltext::match_effect_polarity(&text) {
                    let ts_ms = ts.secs() * 1000;
                    matched.push((
                        ts_ms,
                        Some(Classified::EffectPolarityHit {
                            who: m.target,
                            text: text.to_string(),
                        }),
                    ));
                    true
                } else {
                    false
                };
                // why: a recognized-but-unruled line isn't "Unparsed";
                // only a real miss on both checks gets shape-clustered
                if !recognized {
                    shaper.shape_into(text_bytes, ShapeMode::Aggressive, &mut shape_scratch);
                    if let Some(s) = unmatched_shapes.get_mut(&shape_scratch) {
                        s.count += 1;
                    } else {
                        unmatched_shapes.insert(
                            shape_scratch.clone(),
                            ShapeStat {
                                count: 1,
                                example: text_bytes.to_vec(),
                            },
                        );
                    }
                }
            }
            Outcome::Headerless { .. } => counts.headerless += 1,
            Outcome::Blank => counts.blank += 1,
        }
    }
    ChunkResult {
        counts,
        matched,
        unmatched_shapes,
    }
}

/// why: splits into complete lines, CRLF-tolerant, holding back a
/// trailing partial line; pub so examples/dump_fixtures.rs can frame the same way
pub fn framed_lines(raw: &[u8]) -> Vec<&[u8]> {
    if raw.is_empty() {
        return Vec::new();
    }
    let mut parts: Vec<&[u8]> = raw.split(|&b| b == b'\n').collect();
    // why: last element is trailing empty (after \n) or a partial line -- either way drop it
    parts.pop();
    parts.into_iter().map(strip_cr).collect()
}

fn strip_cr(line: &[u8]) -> &[u8] {
    match line.split_last() {
        Some((&b'\r', rest)) => rest,
        _ => line,
    }
}

/// why: parses `lines` across several threads instead of one at a time.
/// Classification (regex-bound, ~900ns/line) parallelizes across an
/// immutable Send+Sync Engine; applying results can't (encounter graph
/// and zone spans are order-dependent), so that stays one sequential
/// pass. Takes pre-framed lines so tail_worker.rs can hand this one
/// bounded chunk at a time -- keeps Ingest's lock from being held for a
/// multi-million-line file's entire replay with no progress tick in between.
pub fn backfill_lines(ing: &mut Ingest, engine: &Engine, lines: &[&[u8]], threads: usize) {
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

    // why: sequential merge in file order -- hashmap/vec work, no regex left
    for r in results {
        ing.counts.add(&r.counts);
        for (shape, stat) in r.unmatched_shapes {
            ing.merge_unmatched_shape(shape, stat);
        }
        for (ts_ms, item) in r.matched {
            ing.log_clock.set_at_least(ts_ms);
            match item {
                Some(Classified::Action(action)) => ing.apply(ts_ms, action),
                Some(Classified::SelfFlavorHit { classes, text }) => {
                    ing.record_effect_ping(ts_ms, "You", &text);
                    ing.attribute_flavor_hit(ts_ms, &text, classes);
                }
                Some(Classified::ThirdPersonFlavorHit { who, text }) => {
                    ing.record_effect_ping(ts_ms, &who, &text);
                }
                Some(Classified::EffectPolarityHit { who, text }) => {
                    ing.record_effect_ping(ts_ms, &who, &text);
                }
                Some(Classified::SpellEffectHit {
                    spell,
                    target,
                    is_wearsoff,
                }) => {
                    ing.confirm_spell_effect(
                        ts_ms,
                        crate::spelltext::SpellTextMatch {
                            spell,
                            target,
                            is_wearsoff,
                        },
                    );
                }
                None => {}
            }
        }
    }
}

#[cfg(test)]
mod xp_tests {
    use super::*;
    use crate::parser::build_engine;

    /// why: real solo-pet-kill lines; xp line sits between the kill's last two damage lines and its death line
    const KILL_XP: &str = "\
[Tue Jul 28 15:02:08 2026] You are not currently assigned to an adventure.
[Tue Jul 28 15:02:46 2026] Auto attack is on.
[Tue Jul 28 15:02:49 2026] You begin casting Burst of Flame.
[Tue Jul 28 15:02:50 2026] You hit a fragile pet for 3 points of fire damage by Burst of Flame.
[Tue Jul 28 15:02:50 2026] A fragile pet singes as the Burst of Flame hits them.
[Tue Jul 28 15:02:51 2026] A fragile pet tries to punch YOU, but misses!
[Tue Jul 28 15:02:53 2026] You begin casting Burst of Flame.
[Tue Jul 28 15:02:54 2026] A fragile pet punches YOU for 2 points of damage.
[Tue Jul 28 15:02:54 2026] You regain your concentration and continue your casting.
[Tue Jul 28 15:02:54 2026] You gain experience! (11.000%)
[Tue Jul 28 15:02:54 2026] You hit a fragile pet for 3 points of fire damage by Burst of Flame.
[Tue Jul 28 15:02:54 2026] A fragile pet singes as the Burst of Flame hits them.
[Tue Jul 28 15:02:54 2026] You have slain a fragile pet!
[Tue Jul 28 15:02:57 2026] You cannot see your target.
";

    /// why: real quest-turnin lines; same xp line as a kill but no kill nearby
    const QUEST_XP: &str = "\
[Tue Jul 28 15:03:55 2026] You offered 1 Rambunctious Pet's Skull to Dead Doug.
[Tue Jul 28 15:03:57 2026] You offered 1 Fragile Pet's Skull to Dead Doug.
[Tue Jul 28 15:03:58 2026] Dead Doug says, 'Ahh Dougrick, I knew him well.'
[Tue Jul 28 15:03:58 2026] You gain experience! (4.000%)
[Tue Jul 28 15:03:58 2026] You complete the trade with Dead Doug.
";

    fn run(text: &str) -> Ingest {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = text.lines().map(str::as_bytes).collect();
        backfill_lines(&mut ing, &engine, &lines, 1);
        ing
    }

    #[test]
    fn kill_xp_attributes_to_the_kill() {
        let ing = run(KILL_XP);
        let xp_rows: Vec<usize> = (0..ing.store.len())
            .filter(|&i| ing.store.kind[i] == EventKind::Xp)
            .collect();
        assert_eq!(xp_rows.len(), 1, "expected exactly one Xp row");
        let row = xp_rows[0];
        assert_eq!(
            ing.store.amount[row], 11_000,
            "11.000% should store as 11000 milli-percent"
        );
        assert_eq!(ing.store.ability_name(ing.store.ability[row]), "solo");

        let enc = ing.store.enc[row];
        assert_ne!(
            enc, NO_ENCOUNTER,
            "should have linked to the fragile pet's own encounter"
        );
        let e = ing
            .store
            .encounter(EncounterId(enc))
            .expect("linked encounter exists");
        // why: not asserting e.slain -- needs more trailing quiet time
        // than this snippet gives; checking attribution to the right encounter, not close-out timing
        assert!(ing
            .store
            .name(e.target)
            .eq_ignore_ascii_case("a fragile pet"));
    }

    #[test]
    fn quest_turnin_xp_stays_unattributed() {
        let ing = run(QUEST_XP);
        let xp_rows: Vec<usize> = (0..ing.store.len())
            .filter(|&i| ing.store.kind[i] == EventKind::Xp)
            .collect();
        assert_eq!(xp_rows.len(), 1, "expected exactly one Xp row");
        let row = xp_rows[0];
        assert_eq!(ing.store.amount[row], 4_000);
        assert_eq!(
            ing.store.enc[row], NO_ENCOUNTER,
            "no kill nearby -- must not be misattributed to some other fight"
        );
    }
}

#[cfg(test)]
mod currency_tests {
    use super::*;
    use crate::parser::build_engine;

    #[test]
    fn parses_every_real_separator_style() {
        // Every distinct shape confirmed present in the real reference
        // log -- see money.vendor_sell/money.corpse/loot.self.direct's
        // own doc comments in eql.toml for where each comes from.
        assert_eq!(
            parse_currency_copper("2 platinum, 5 gold, 7 silver and 2 copper"),
            2000 + 500 + 70 + 2
        );
        assert_eq!(
            parse_currency_copper("9 platinum 5 gold 7 silver"),
            9000 + 500 + 70
        );
        assert_eq!(parse_currency_copper("185 platinum"), 185_000);
        assert_eq!(parse_currency_copper("3 silver and 4 copper"), 30 + 4);
        assert_eq!(parse_currency_copper("1 silver"), 10);
        assert_eq!(parse_currency_copper("4 copper"), 4);
        assert_eq!(parse_currency_copper(""), 0);
        assert_eq!(parse_currency_copper("nothing recognisable here"), 0);
    }

    fn run(text: &str) -> Ingest {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = text.lines().map(str::as_bytes).collect();
        backfill_lines(&mut ing, &engine, &lines, 1);
        ing
    }

    fn currency_rows(ing: &Ingest) -> Vec<(u64, String)> {
        (0..ing.store.len())
            .filter(|&i| ing.store.kind[i] == EventKind::Currency)
            .map(|i| {
                (
                    ing.store.amount[i],
                    ing.store.ability_name(ing.store.ability[i]).to_string(),
                )
            })
            .collect()
    }

    #[test]
    fn corpse_currency_is_recorded() {
        // Real line, eqlog_Manipulator_rivervale.txt:81.
        let ing =
            run("[Tue Jul 28 15:03:22 2026] You receive 3 silver and 4 copper from the corpse.\n");
        assert_eq!(currency_rows(&ing), vec![(34, "corpse".to_string())]);
    }

    #[test]
    fn vendor_sell_currency_is_recorded() {
        // Real line, eqlog_Manipulator_rivervale.txt (Klok Koglin trade).
        let ing = run("[Tue Jul 28 15:02:15 2026] You receive 9 platinum 5 gold 7 silver from Klok Koglin for the Gold Malachite Bracelet(s).\n");
        assert_eq!(currency_rows(&ing), vec![(9570, "vendor".to_string())]);
    }

    #[test]
    fn autosell_currency_rides_along_with_the_loot_row() {
        // Real line, eqlog_Manipulator_rivervale.txt:33761 -- one line,
        // two facts: an item looted *and* platinum earned.
        let ing = run("[Tue Jul 28 20:01:53 2026] You looted a Snake Venom Sac from a giant snake's corpse and sold it for 3 gold, 5 silver and 7 copper.\n");
        let loot_rows: Vec<usize> = (0..ing.store.len())
            .filter(|&i| ing.store.kind[i] == EventKind::Loot)
            .collect();
        assert_eq!(
            loot_rows.len(),
            1,
            "the item itself should still be recorded"
        );
        assert_eq!(currency_rows(&ing), vec![(357, "autosell".to_string())]);
    }
}

#[cfg(test)]
mod afk_tests {
    use super::*;
    use crate::parser::build_engine;

    /// why: real lines confirming session_start prefers AFK-off over first_ts once one exists
    const AFK_ROUND_TRIP: &str = "\
[Tue Jul 28 15:02:08 2026] You are not currently assigned to an adventure.
[Sun Aug 16 19:43:02 2026] You are now A.F.K. (Away From Keyboard).
[Sun Aug 16 19:43:03 2026] You are no longer A.F.K. (Away From Keyboard).
";

    #[test]
    fn session_start_follows_the_most_recent_afk_off() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = AFK_ROUND_TRIP.lines().map(str::as_bytes).collect();
        backfill_lines(&mut ing, &engine, &lines, 1);
        assert!(!ing.currently_afk(), "the round trip ends AFK-off");
        // why: well after file start -- proves session_start picked the AFK-off line, not first_ts
        let start = ing.session_start().expect("an afk.off line was seen");
        let first = ing.first_ts.expect("at least one line was processed");
        assert!(start > first, "session_start should have moved to the afk-off timestamp, not stayed at the file's own start");
    }

    #[test]
    fn still_afk_leaves_the_previous_session_start_alone() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> =
            "[Sun Aug 16 19:43:02 2026] You are now A.F.K. (Away From Keyboard).\n"
                .lines()
                .map(str::as_bytes)
                .collect();
        backfill_lines(&mut ing, &engine, &lines, 1);
        assert!(ing.currently_afk());
        // why: no afk.off seen -- falls back to first_ts, not None or the afk.on line
        assert_eq!(ing.session_start(), ing.first_ts);
    }
}

#[cfg(test)]
mod aa_tests {
    use super::*;
    use crate::parser::build_engine;

    /// why: real lines covering first purchase, rank-up, free rank, and
    /// the plural/singular "point(s)" grammar wrinkle distinguishing
    /// aa.gained from aa.improved; innate-skill-grant line must not be picked up as AA
    const AA_LINES: &str = "\
[Fri Jul 31 16:55:33 2026] You have gained the ability \"Spell Casting Deftness\" at a cost of 2 ability points.
[Fri Aug 07 00:25:51 2026] You have gained the ability \"Unbound Drain\" at a cost of 0 ability points.
[Sat Aug 08 00:36:12 2026] You have gained the ability to use Double Attack.
[Mon Aug 10 09:00:00 2026] You have improved Spell Casting Deftness 2 at a cost of 4 ability points.
[Fri Aug 07 21:00:06 2026] You have improved Innate Regeneration 2 at a cost of 1 ability point.
";

    #[test]
    fn real_aa_lines_are_recorded_and_the_innate_skill_grant_is_not() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = AA_LINES.lines().map(str::as_bytes).collect();
        backfill_lines(&mut ing, &engine, &lines, 1);

        let grants: Vec<&AaGrant> = ing.aa.all().map(|(_, g)| g).collect();
        // why: 4 real lines went in -- innate-skill-grant line must not add a 5th
        assert_eq!(grants.len(), 4);

        assert_eq!(grants[0].name, "Spell Casting Deftness");
        assert_eq!(grants[0].rank, 1); // why: aa.gained always synthesizes rank 1
        assert_eq!(grants[0].cost, 2);

        assert_eq!(grants[1].name, "Unbound Drain");
        assert_eq!(grants[1].cost, 0); // why: a free first rank is still a real grant

        assert_eq!(grants[2].name, "Spell Casting Deftness");
        assert_eq!(grants[2].rank, 2); // why: aa.improved's own rank digit, not synthesized
        assert_eq!(grants[2].cost, 4);

        assert_eq!(grants[3].name, "Innate Regeneration");
        assert_eq!(grants[3].rank, 2);
        assert_eq!(grants[3].cost, 1); // why: singular "1 ability point." still parses

        #[allow(clippy::identity_op)]
        // why: +0 kept -- lines up 1:1 with grants[0..3]'s costs above
        {
            assert_eq!(ing.aa.total_spent(), 2 + 0 + 4 + 1);
        }
    }
}

#[cfg(test)]
mod exaltation_proc_tests {
    use super::*;
    use crate::parser::build_engine;

    /// why: real lines, two items each firing more than once; only pins
    /// the Exaltation case since no non-Exaltation example exists in the real log
    const PROC_LINES: &str = "\
[Thu Jul 30 05:51:35 2026] Your Flowing Black Robe (Exaltation) flickers with a pale light.
[Thu Jul 30 06:04:36 2026] Your Flowing Black Robe (Exaltation) flickers with a pale light.
[Tue Jul 28 15:02:15 2026] Your Black Tome with Silver Runes (Exaltation) feels alive with power.
[Thu Jul 30 16:49:38 2026] Your Flowing Black Robe (Exaltation) flickers with a pale light.
";

    #[test]
    fn exaltation_proc_lines_are_tallied_per_item() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = PROC_LINES.lines().map(str::as_bytes).collect();
        backfill_lines(&mut ing, &engine, &lines, 1);

        assert_eq!(ing.exaltation_procs.count("Flowing Black Robe"), 3);
        assert_eq!(
            ing.exaltation_procs.count("Black Tome with Silver Runes"),
            1
        );
        assert_eq!(ing.exaltation_procs.count("Something Never Seen"), 0);
    }

    /// why: observe's contract -- first call's timestamp sticks, a later
    /// repeat only bumps the count, never overwrites
    #[test]
    fn first_seen_sticks_to_the_first_observation_not_a_later_one() {
        let mut procs = ExaltationProcs::default();
        procs.observe(1_000, "Flowing Black Robe".to_string());
        procs.observe(5_000, "Flowing Black Robe".to_string());
        procs.observe(9_000, "Flowing Black Robe".to_string());

        assert_eq!(procs.count("Flowing Black Robe"), 3);
        assert_eq!(procs.first_seen_ms("Flowing Black Robe"), Some(1_000));
    }

    #[test]
    fn an_item_with_no_evidence_reads_as_zero_not_missing() {
        let procs = ExaltationProcs::default();
        assert_eq!(procs.count("Never Fired"), 0);
        assert_eq!(procs.first_seen_ms("Never Fired"), None);
    }
}

#[cfg(test)]
mod spell_tests {
    use super::*;
    use crate::parser::build_engine;

    /// why: real lines -- one spell memorized twice (gem-swap
    /// re-memorize), one once, and a begin/forget pair that must not register as known
    const SPELL_LINES: &str = "\
[Tue Jul 28 17:10:37 2026] Beginning to memorize Color Flux...
[Tue Jul 28 17:10:46 2026] You have finished memorizing Color Flux.
[Tue Jul 28 17:43:03 2026] Beginning to memorize Suffocating Sphere...
[Tue Jul 28 17:43:10 2026] You have finished memorizing Suffocating Sphere.
[Tue Jul 28 18:00:00 2026] You forget Color Flux.
[Tue Jul 28 18:00:05 2026] Beginning to memorize Color Flux...
[Tue Jul 28 18:00:12 2026] You have finished memorizing Color Flux.
[Tue Jul 28 19:00:00 2026] Beginning to memorize Ice Spear...
";

    #[test]
    fn confirmed_memorizes_are_recorded_deduped_to_first_sighting() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = SPELL_LINES.lines().map(str::as_bytes).collect();
        backfill_lines(&mut ing, &engine, &lines, 1);

        let known: std::collections::HashMap<&str, Millis> = ing.spellbook.known().collect();
        // why: Ice Spear only ever began memorizing, never finished -- must not appear as Known
        assert_eq!(known.len(), 2);
        assert!(known.contains_key("Suffocating Sphere"));

        // why: Ice Spear is the Possible tier's whole reason to exist -- begin with no finish
        let possible: std::collections::HashMap<&str, Millis> = ing.spellbook.possible().collect();
        assert_eq!(possible.len(), 1);
        assert!(possible.contains_key("Ice Spear"));
        assert!(!known.contains_key("Ice Spear"));

        // why: completed twice -- first_seen must stay pinned to the first completion
        let mut first_only = Ingest::default();
        let first_line: Vec<&[u8]> =
            vec![b"[Tue Jul 28 17:10:46 2026] You have finished memorizing Color Flux."];
        backfill_lines(&mut first_only, &engine, &first_line, 1);
        let expected_first_ts = first_only
            .spellbook
            .known()
            .next()
            .expect("one line, one grant")
            .1;
        assert_eq!(known["Color Flux"], expected_first_ts);
    }

    /// why: scribing is now the primary "added to spellbook" signal
    /// (596/593 real occurrences), not just a memorize fallback; proves scribe has its own Possible tier too
    #[test]
    fn scribing_a_new_scroll_reaches_known_the_same_way_memorizing_does() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        const SCRIBE_LINES: &str = "\
[Wed Jul 29 17:02:20 2026] Beginning to scribe Project Lightning...
[Wed Jul 29 17:02:20 2026] You have finished scribing Project Lightning.
[Wed Jul 29 17:02:21 2026] Beginning to scribe Pillar of Fire...
[Wed Jul 29 17:02:22 2026] Beginning to scribe Levitate...
[Wed Jul 29 17:02:22 2026] You have finished scribing Levitate.
";
        let lines: Vec<&[u8]> = SCRIBE_LINES.lines().map(str::as_bytes).collect();
        backfill_lines(&mut ing, &engine, &lines, 1);

        let known: std::collections::HashMap<&str, Millis> = ing.spellbook.known().collect();
        let possible: std::collections::HashMap<&str, Millis> = ing.spellbook.possible().collect();
        assert!(known.contains_key("Project Lightning"));
        assert!(known.contains_key("Levitate"));
        assert!(possible.contains_key("Pillar of Fire"));
        assert!(
            !known.contains_key("Pillar of Fire"),
            "Pillar of Fire never got a finish line in this excerpt"
        );
    }

    /// why: began via memorize, finished via scribe -- both prove
    /// spellbook membership, still reaches Known, first_began stays pinned to the memorize attempt
    #[test]
    fn began_memorizing_then_finished_scribing_still_reaches_known() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        const LINES: &str = "\
[Wed Jul 29 17:02:20 2026] Beginning to memorize Levitate...
[Wed Jul 29 17:02:25 2026] Beginning to scribe Levitate...
[Wed Jul 29 17:02:26 2026] You have finished scribing Levitate.
";
        let lines: Vec<&[u8]> = LINES.lines().map(str::as_bytes).collect();
        backfill_lines(&mut ing, &engine, &lines, 1);

        assert_eq!(ing.spellbook.known().count(), 1);
        assert_eq!(ing.spellbook.possible().count(), 0);
    }

    /// why: throwaway sanity-check of Known/Possible counts against the
    /// full reference log; machine-local path, not a permanent test
    #[test]
    #[ignore]
    fn cross_check_against_the_real_reference_log() {
        let raw = std::fs::read("/home/Spencer/eqlp/eqlog_Manipulator_rivervale.txt")
            .expect("reference log present");
        let lines = framed_lines(&raw);
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        backfill_lines(&mut ing, &engine, &lines, 8);
        let known = ing.spellbook.known().count();
        let possible = ing.spellbook.possible().count();
        println!("known: {known}, possible: {possible}");
        assert!(
            known > 100,
            "a 12-day-spanning log should have plenty of confirmed spells"
        );
        assert!(
            possible < known,
            "most attempts should complete -- possible is meant to be the small tail"
        );
    }
}

#[cfg(test)]
mod unmatched_shape_tests {
    use super::*;
    use crate::parser::build_engine;

    /// why: real lines -- a genuinely-unrecognized pair that must
    /// collapse to one shape, two flavor-recognized lines that must never
    /// show up here, one matched line unaffected; 4 threads on 6 lines
    /// forces multiple chunks, exercising the per-thread merge
    const LINES: &str = "\
[Tue Jul 28 15:02:14 2026] Xscyte's hand is covered with a nonexistent aura.
[Tue Jul 28 15:02:15 2026] Harli's hand is covered with a nonexistent aura.
[Tue Jul 28 15:02:16 2026] The jig sends energy zinging through your body.
[Tue Jul 28 15:02:17 2026] Deathklokk's voice booms.
[Tue Jul 28 15:02:18 2026] Moxie's voice booms.
[Tue Jul 28 15:02:19 2026] You have slain a fragile pet!
";

    #[test]
    fn real_unmatched_lines_cluster_into_shapes_across_chunks() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = LINES.lines().map(str::as_bytes).collect();
        backfill_lines(&mut ing, &engine, &lines, 4);

        // why: counts every rule-engine miss regardless of flavor-recognition -- pattern match, not understanding
        assert_eq!(ing.counts.unmatched, 5); // nonexistent-aura x2 + jig + voice-booms x2
        assert_eq!(ing.counts.matched, 1); // death.you_slew
        assert_eq!(
            ing.unmatched_shapes_distinct(),
            1,
            "only the genuinely-unrecognized nonexistent-aura pair should cluster here"
        );
        assert_eq!(ing.unmatched_shapes_overflow(), 0);

        let top = ing.unmatched_shapes_top(10);
        let total_count: u64 = top.iter().map(|(_, s)| s.count).sum();
        assert_eq!(total_count, 2);

        let examples: Vec<String> = top
            .iter()
            .map(|(_, s)| String::from_utf8_lossy(&s.example).into_owned())
            .collect();
        assert!(
            examples.iter().any(|e| e.contains("nonexistent aura")),
            "the genuinely-unrecognized pair should be kept as the example"
        );
        assert!(
            !examples
                .iter()
                .any(|e| e.contains("jig") || e.ends_with("voice booms.")),
            "flavor-recognized lines are understood -- they must not appear as unparsed"
        );
        assert!(
            !examples.iter().any(|e| e.contains("slain")),
            "the matched death line must never appear as an unmatched shape"
        );
    }

    /// why: same exclusion via the live-tail path, a separate code path with its own copy of this logic
    #[test]
    fn a_flavor_recognized_line_is_excluded_from_unparsed_on_the_live_path_too() {
        let engine = build_engine().expect("pack builds");
        let mut matcher = engine.matcher();
        let mut ing = Ingest::default();
        for line in [
            &b"[Tue Jul 28 15:02:14 2026] Xscyte's hand is covered with a nonexistent aura."[..],
            &b"[Tue Jul 28 15:02:16 2026] The jig sends energy zinging through your body."[..],
            &b"[Tue Jul 28 15:02:17 2026] Deathklokk's voice booms."[..],
        ] {
            let outcome = matcher.classify(line);
            ing.route(&engine, line, &outcome);
        }

        assert_eq!(ing.counts.unmatched, 3);
        assert_eq!(
            ing.unmatched_shapes_distinct(),
            1,
            "only the genuinely-unrecognized nonexistent-aura line should show up"
        );
        let top = ing.unmatched_shapes_top(10);
        assert!(top
            .iter()
            .any(|(_, s)| String::from_utf8_lossy(&s.example).contains("nonexistent aura")));
        assert!(
            !top.iter().any(|(_, s)| {
                let e = String::from_utf8_lossy(&s.example);
                e.contains("jig") || e.ends_with("voice booms.")
            }),
            "flavor-recognized lines must not appear as unparsed on the live path either"
        );
    }

    /// why: throwaway cross-check against `eqlp coverage`'s real
    /// numbers on the full reference log; machine-local path
    #[test]
    #[ignore]
    fn cross_check_against_the_real_reference_log() {
        let raw = std::fs::read("/home/Spencer/eqlp/eqlog_Manipulator_rivervale.txt")
            .expect("reference log present");
        let lines = framed_lines(&raw);
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        backfill_lines(&mut ing, &engine, &lines, 8);
        println!("distinct: {}", ing.unmatched_shapes_distinct());
        println!("overflow: {}", ing.unmatched_shapes_overflow());
        let top = ing.unmatched_shapes_top(5);
        for (shape, stat) in &top {
            println!("{:>10}  {}", stat.count, String::from_utf8_lossy(shape));
        }

        // why: overflow count isn't reproducible bit-for-bit across a
        // parallel merge (HashMap order != file order) -- the real
        // invariant is every unmatched line counted exactly once
        let tracked_total: u64 = ing
            .unmatched_shapes_top(usize::MAX)
            .iter()
            .map(|(_, s)| s.count)
            .sum();
        assert_eq!(
            tracked_total + ing.unmatched_shapes_overflow(),
            ing.counts.unmatched
        );

        // why: order-insensitive facts -- matched `eqlp coverage --top 5`'s real output exactly
        assert_eq!(
            ing.unmatched_shapes_distinct(),
            4096,
            "the cap should still fill exactly, order or not"
        );
        assert_eq!(
            top[0].1.count, 7824,
            "the single most common shape is unambiguous regardless of merge order"
        );
        assert_eq!(
            String::from_utf8_lossy(top[0].0),
            "The jig sends energy zinging through your body."
        );
    }
}

#[cfg(test)]
mod notification_wiring_tests {
    use super::*;
    use crate::parser::build_engine;

    /// why: checks the live wiring -- route pushes notifications only
    /// when live, not during backfill (no sound burst replaying history)
    #[test]
    fn route_collects_notifications_only_when_live() {
        let engine = build_engine().expect("pack builds");
        let mut matcher = engine.matcher();
        let line: &[u8] =
            b"[Tue Jul 28 15:02:15 2026] You have gained a level! Welcome to level 2!";

        let mut backfilled = Ingest::default();
        let outcome = matcher.classify(line);
        backfilled.route(&engine, line, &outcome);
        assert!(
            backfilled.pending_notifications.is_empty(),
            "not live -- must not queue a notification"
        );

        let mut live = Ingest::default();
        live.mark_live();
        let outcome = matcher.classify(line);
        live.route(&engine, line, &outcome);
        assert_eq!(live.pending_notifications.len(), 1);
        assert_eq!(
            live.pending_notifications[0].kind,
            crate::notifications::LEVEL_UP
        );
        assert_eq!(live.pending_notifications[0].message, "Level 2!");
    }
}

#[cfg(test)]
mod newly_recognized_line_tests {
    use super::*;
    use crate::parser::build_engine;

    /// why: each real line pulled from the unmatched-shape backlog now matches its own rule
    #[test]
    fn each_line_matches_its_own_rule() {
        let engine = build_engine().expect("pack builds");
        let mut matcher = engine.matcher();
        let cases: &[(&[u8], &str)] = &[
            (b"[Tue Jul 28 15:02:15 2026] LOADING, PLEASE WAIT...", "noise.loading"),
            (b"[Tue Jul 28 15:02:15 2026] You have been poisoned.", "state.you_poisoned"),
            (b"[Tue Jul 28 15:02:15 2026] a rattlesnake has been poisoned.", "state.poisoned"),
            (
                b"[Tue Jul 28 15:02:15 2026] Your Allure spell on an abhorrent has been overwritten.",
                "state.spell_overwritten",
            ),
            (
                b"[Tue Jul 28 15:02:15 2026] Refuse tries to punch Refugee Splitpaw, but Refugee Splitpaw blocks!",
                "melee.blocked",
            ),
            (
                b"[Tue Jul 28 15:02:15 2026] Refuse tries to kick Refugee Splitpaw, but Refugee Splitpaw blocks!",
                "melee.blocked",
            ),
            (b"[Tue Jul 28 15:02:15 2026] You assume an evasive stance.", "state.stance"),
            (
                b"[Tue Jul 28 15:02:15 2026] Ice boned skeleton tries to punch YOU, but YOU dodge!",
                "melee.dodged",
            ),
            (
                b"[Tue Jul 28 15:02:15 2026] A leering gargoyle tries to hit Bravesirrobin, but Bravesirrobin dodges!",
                "melee.dodged",
            ),
            (
                b"[Tue Jul 28 15:02:15 2026] You begin reciting the arcane mastery invocation.",
                "state.invocation",
            ),
            (b"[Tue Jul 28 15:02:15 2026] You begin to change your invocation.", "noise.invocation_menu"),
            (
                b"[Fri Aug 07 00:00:07 2026] Orc scoutsman has taken 104 damage from your Elemental Maelstrom X.",
                "dot.damage_from_you",
            ),
            (
                b"[Sun Aug 09 04:21:39 2026] Orc legionnaire has taken 16 damage from your Leech. (Critical)",
                "dot.damage_from_you",
            ),
            (
                b"[Fri Aug 14 21:11:25 2026] Your Berserker Strength spell did not take hold on Hakujin. (Blocked by Berserker Spirit.)",
                "cast.blocked",
            ),
            (
                b"[Sat Aug 15 09:49:02 2026] Your Shield of Lava spell did not take hold on Bravesirrobin.",
                "cast.blocked",
            ),
            (
                b"[Fri Aug 07 16:39:00 2026] Stand close to and right click on the Player to inspect him. Use the /toggleinspect command to enable or disable right-click inspecting.",
                "noise.right_click_hint",
            ),
            (
                b"[Fri Aug 07 16:43:07 2026] Stand close to and right click on the Merchant to begin a transaction.",
                "noise.right_click_hint",
            ),
            (b"[Fri Aug 07 19:38:37 2026] You have been diseased.", "state.you_diseased"),
            (b"[Fri Aug 07 19:52:48 2026] Dojii has been diseased.", "state.diseased"),
            (b"[Fri Aug 07 00:47:50 2026] Spell set WED_29_Grind loaded.", "noise.spell_set_loaded"),
            (
                b"[Fri Aug 14 16:32:24 2026] There are no open slots for the held item in your inventory.",
                "noise.no_open_slots",
            ),
            (
                b"[Fri Aug 07 16:58:14 2026] Your target is out of range, get closer!",
                "noise.out_of_range",
            ),
            (b"[Fri Aug 07 16:36:07 2026] You are missing Platinum Bar.", "noise.missing_component"),
            (
                b"[Fri Aug 07 16:36:07 2026] You are missing some required components.",
                "noise.missing_component",
            ),
            (
                b"[Fri Aug 07 17:10:56 2026] You cannot switch invocations while casting!",
                "noise.invocation_switch_blocked",
            ),
            (
                b"[Tue Aug 18 22:28:36 2026] Your Location is 216.51, -103.09, -20.19",
                "state.location",
            ),
        ];
        for (line, expected_id) in cases {
            let outcome = matcher.classify(line);
            let eqlp_core::Outcome::Matched(m) = outcome else {
                panic!("should have matched: {}", String::from_utf8_lossy(line));
            };
            assert_eq!(engine.rule(m.rule).id.as_str(), *expected_id);
        }
    }

    /// why: an avoided swing lands on the same ability row as a landed
    /// one, tagged by avoidance kind, not a synthetic "Miss" ability; attempts() is the honest denominator
    #[test]
    fn avoided_swings_land_on_the_same_ability_row_as_a_real_hit_of_that_type() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Tue Jul 28 15:02:10 2026] Refuse punches Refugee Splitpaw for 5 points of damage.",
            b"[Tue Jul 28 15:02:11 2026] Refuse tries to punch Refugee Splitpaw, but Refugee Splitpaw blocks!",
            b"[Tue Jul 28 15:02:12 2026] Refuse tries to punch Refugee Splitpaw, but misses!",
            b"[Tue Jul 28 15:02:13 2026] Ice boned skeleton tries to punch YOU, but YOU dodge!",
            b"[Tue Jul 28 15:02:14 2026] a rattlesnake tries to slash Vonektik, but Vonektik parries!",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let refuse = ing
            .store
            .names
            .get("Refuse")
            .expect("Refuse should be interned");
        let punch_rows = by_ability(&ing.store, &Filter::default().by(refuse));
        let punch = punch_rows
            .iter()
            .find(|r| ing.store.ability_name(r.ability) == "Punch")
            .expect("a Punch row should exist");
        assert_eq!(punch.hits, 1, "the one landed punch");
        assert_eq!(punch.total, 5);
        assert_eq!(punch.blocked, 1);
        assert_eq!(punch.missed, 1);
        assert_eq!(punch.dodged, 0);
        assert_eq!(punch.attempts(), 3, "1 landed + 1 blocked + 1 missed");

        let skeleton = ing
            .store
            .names
            .get("Ice boned skeleton")
            .expect("skeleton should be interned");
        let skel_rows = by_ability(&ing.store, &Filter::default().by(skeleton));
        let skel_punch = skel_rows
            .iter()
            .find(|r| ing.store.ability_name(r.ability) == "Punch")
            .expect("a Punch row should exist");
        assert_eq!(skel_punch.dodged, 1);
        assert_eq!(skel_punch.hits, 0, "never actually landed");

        let snake = ing
            .store
            .names
            .get("a rattlesnake")
            .expect("rattlesnake should be interned");
        let snake_rows = by_ability(&ing.store, &Filter::default().by(snake));
        let snake_slash = snake_rows
            .iter()
            .find(|r| ing.store.ability_name(r.ability) == "Slash")
            .expect("a Slash row should exist");
        assert_eq!(snake_slash.parried, 1);
    }

    /// why: real bug -- avoidance rules hard-anchored !$ so a flagged
    /// miss/block/dodge/parry (Riposte/Rampage/Flurry) fell through to
    /// unmatched entirely; now each lands on its ability row and keeps the special-attack bit
    #[test]
    fn a_flagged_avoidance_still_counts_the_swing_and_keeps_its_own_flag() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Fri Aug 07 00:00:07 2026] Socho Darkpaw tries to hit Bravesirrobin, but misses! (Riposte)",
            b"[Fri Aug 07 00:00:08 2026] Socho Darkpaw tries to hit Bravesirrobin, but Bravesirrobin blocks! (Rampage)",
            b"[Fri Aug 07 00:00:09 2026] Socho Darkpaw tries to hit Bravesirrobin, but Bravesirrobin dodges! (Flurry)",
            b"[Fri Aug 07 00:00:10 2026] Socho Darkpaw tries to hit Bravesirrobin, but Bravesirrobin parries! (Riposte)",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let socho = ing
            .store
            .names
            .get("Socho Darkpaw")
            .expect("Socho Darkpaw should be interned");
        let rows = by_ability(&ing.store, &Filter::default().by(socho));
        let hit = rows
            .iter()
            .find(|r| ing.store.ability_name(r.ability) == "Hit")
            .expect("a Hit row should exist");
        assert_eq!(hit.missed, 1);
        assert_eq!(hit.blocked, 1);
        assert_eq!(hit.dodged, 1);
        assert_eq!(hit.parried, 1);
        assert_eq!(
            hit.attempts(),
            4,
            "none of the 4 flagged swings should be lost"
        );

        // why: confirm the flag itself, not just that the swing landed on a row
        let miss_flags: Vec<eqlp_store::Flags> = (0..ing.store.kind.len())
            .filter(|&i| {
                ing.store.kind[i] == eqlp_store::EventKind::Miss && ing.store.actor[i] == socho
            })
            .map(|i| ing.store.flags[i])
            .collect();
        assert_eq!(miss_flags.len(), 4);
        assert!(
            miss_flags[0] & eqlp_store::flag::RIPOSTE != 0,
            "{:#x}",
            miss_flags[0]
        );
        assert!(
            miss_flags[1] & eqlp_store::flag::RAMPAGE != 0,
            "{:#x}",
            miss_flags[1]
        );
        assert!(
            miss_flags[2] & eqlp_store::flag::FLURRY != 0,
            "{:#x}",
            miss_flags[2]
        );
        assert!(
            miss_flags[3] & eqlp_store::flag::RIPOSTE != 0,
            "{:#x}",
            miss_flags[3]
        );
    }
}

#[cfg(test)]
mod zone_change_encounter_tests {
    use super::*;
    use crate::parser::build_engine;

    fn run(text: &str) -> Ingest {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = text.lines().map(str::as_bytes).collect();
        backfill_lines(&mut ing, &engine, &lines, 1);
        ing
    }

    /// why: same mob name after zoning must not reuse the old encounter
    #[test]
    fn a_same_named_mob_after_zoning_starts_a_fresh_encounter_not_the_old_one() {
        let log = "\
[Tue Jul 28 15:02:08 2026] You hit a gnoll for 5 points of damage.
[Tue Jul 28 15:02:10 2026] You have entered Blackburrow.
[Tue Jul 28 15:02:15 2026] You hit a gnoll for 7 points of damage.
";
        let ing = run(log);

        let damage_rows: Vec<usize> = (0..ing.store.len())
            .filter(|&i| ing.store.kind[i] == EventKind::Damage)
            .collect();
        assert_eq!(damage_rows.len(), 2, "both hits should be real damage rows");

        let enc_before = ing.store.enc[damage_rows[0]];
        let enc_after = ing.store.enc[damage_rows[1]];
        assert_ne!(
            enc_before, NO_ENCOUNTER,
            "the pre-zone hit should have opened a real encounter"
        );
        assert_ne!(
            enc_after, NO_ENCOUNTER,
            "the post-zone hit should have opened a real encounter"
        );
        assert_ne!(
            enc_before, enc_after,
            "a same-named mob after zoning must be a fresh encounter, not the pre-zone one"
        );

        // why: must end at last action, not the zone-enter line
        let first_hit_ts = ing.store.ts[damage_rows[0]];
        let closed = ing
            .store
            .encounter(EncounterId(enc_before))
            .expect("the pre-zone encounter closed and is in the store");
        assert_eq!(
            closed.end_ms,
            Some(first_hit_ts),
            "should end at the last real action's own timestamp, not the (later) zone-enter line's"
        );
    }
}

#[cfg(test)]
mod stance_evidence_tests {
    use super::*;
    use crate::parser::build_engine;

    /// why: a stance's class list feeds classdetect like an unambiguous
    /// spell -- two real stance lines on two zone visits confirm Berserker, no cast needed
    #[test]
    fn an_unambiguous_stance_confirms_its_one_class() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Tue Jul 28 15:01:00 2026] You have entered Blackburrow.",
            b"[Tue Jul 28 15:01:01 2026] You assume a berserker stance.",
            b"[Tue Jul 28 15:02:00 2026] You have entered West Karana.",
            b"[Tue Jul 28 15:02:01 2026] You assume a berserker stance.",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let you = ing.store.names.get("You").expect("You should be interned");
        let configured = ing
            .classes
            .configuration_of_visit(you.0, ing.zone.index_at(ing.now_ms()));
        assert!(
            configured.contains(&"Berserker".to_string()),
            "{configured:?}"
        );
    }

    /// why: mirror case -- one occurrence isn't enough evidence, same bar a spell is held to
    #[test]
    fn a_single_stance_line_is_not_enough_evidence_yet() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![b"[Tue Jul 28 15:01:00 2026] You assume a berserker stance."];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let you = ing.store.names.get("You").expect("You should be interned");
        let configured = ing
            .classes
            .configuration_of_visit(you.0, ing.zone.index_at(ing.now_ms()));
        assert!(
            !configured.contains(&"Berserker".to_string()),
            "{configured:?}"
        );
    }
}

#[cfg(test)]
mod chat_tests {
    use super::*;
    use crate::parser::build_engine;

    fn run(lines: &[&[u8]]) -> Ingest {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        backfill_lines(&mut ing, &engine, lines, 1);
        ing
    }

    #[test]
    fn guild_party_and_raid_chat_land_in_their_own_channels() {
        let ing = run(&[
            b"[Tue Jul 28 17:09:40 2026] Kaeus tells the guild, 'hi'",
            b"[Wed Jul 29 17:07:48 2026] You tell your party, 'incoming'",
            b"[Wed Aug 05 21:44:57 2026] Mits tells the raid, 'I'm down for D0-D4'",
        ]);
        assert_eq!(ing.chat.guild().len(), 1);
        assert_eq!(ing.chat.guild()[0].who, "Kaeus");
        assert_eq!(ing.chat.guild()[0].text, "hi");
        assert_eq!(ing.chat.party().len(), 1);
        assert_eq!(ing.chat.party()[0].who, "You");
        assert_eq!(ing.chat.raid().len(), 1);
        assert_eq!(ing.chat.raid()[0].who, "Mits");
    }

    /// why: real gap found live -- 422 real "tells the raid" lines in the
    /// reference log matched no rule at all before chat.directed's own
    /// `chan` alternation grew a raid phrasing
    #[test]
    fn raid_chat_is_not_dropped_as_unmatched() {
        let ing = run(&[
            b"[Wed Aug 05 21:44:57 2026] Mits tells the raid, 'yo'",
            b"[Wed Aug 05 21:45:11 2026] You tell your raid, 'yo!'",
        ]);
        assert_eq!(ing.chat.raid().len(), 2);
    }

    /// why: an incoming and an outgoing PM with the same real player must
    /// land in the *same* thread, not two separate ones -- ChatChannel::Pm
    /// keys on the other side regardless of who sent which line
    #[test]
    fn incoming_and_outgoing_pms_with_the_same_player_share_one_thread() {
        let ing = run(&[
            b"[Thu Jul 30 18:04:38 2026] Kaeus tells you, 'busy right now'",
            b"[Thu Jul 30 22:47:34 2026] You told Kaeus, 'no worries'",
        ]);
        let history = ing.chat.pm_history("Kaeus");
        assert_eq!(history.len(), 2, "{history:?}");
        assert_eq!(history[0].who, "Kaeus");
        assert_eq!(history[0].text, "busy right now");
        assert_eq!(history[1].who, "You");
        assert_eq!(history[1].text, "no worries");
    }

    #[test]
    fn pm_threads_reports_one_row_per_real_partner() {
        let ing = run(&[
            b"[Thu Jul 30 18:04:38 2026] Kaeus tells you, 'hi'",
            b"[Thu Jul 30 18:05:00 2026] Opticon tells you, 'yo'",
        ]);
        let mut names: Vec<&str> = ing.chat.pm_threads().map(|(name, _)| name).collect();
        names.sort();
        assert_eq!(names, vec!["Kaeus", "Opticon"]);
    }

    /// why: says/shouts/auctions/OOC are real chat too, but not player-to-
    /// player Social channels -- must not leak into any of the 4 buckets
    #[test]
    fn say_and_auction_do_not_land_in_any_social_channel() {
        let ing = run(&[
            b"[Tue Jul 28 15:02:15 2026] You say, 'send'",
            b"[Tue Jul 28 15:02:15 2026] Biscuits auctions, 'wtb thumper +4'",
        ]);
        assert!(ing.chat.guild().is_empty());
        assert!(ing.chat.party().is_empty());
        assert!(ing.chat.raid().is_empty());
        assert_eq!(ing.chat.pm_threads().count(), 0);
    }
}

#[cfg(test)]
mod unreliable_class_evidence_tests {
    use super::*;
    use crate::parser::build_engine;

    /// why: real bug, caught live -- "Ring of Butcherblock" (Druid-only per
    /// spell_classes.json) showed up cast inside a visit already rock-solid
    /// confirmed as Cleric/Paladin/Shaman, mathematically impossible under
    /// the fixed-CLASS_COUNT rule if the cast genuinely required Druid
    /// active. Group teleports in this game can be triggered as a party
    /// ritual, not gated to the caster's own active classes -- so a
    /// teleport cast must feed classdetect nothing at all, unlike a
    /// regular spell of the same class.
    #[test]
    fn a_known_teleport_spell_contributes_no_class_evidence() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Tue Jul 28 15:01:00 2026] You have entered Blackburrow.",
            b"[Tue Jul 28 15:01:01 2026] You begin casting Ring of Lavastorm.",
            b"[Tue Jul 28 15:02:00 2026] You have entered West Karana.",
            b"[Tue Jul 28 15:02:01 2026] You begin casting Ring of Lavastorm.",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let you = ing.store.names.get("You").expect("You should be interned");
        assert!(
            ing.classes.configurations_of(you.0).is_empty(),
            "a known teleport, cast twice, must still confirm nothing"
        );
    }

    /// why: control case -- an ordinary Druid-only spell (not a teleport)
    /// still confirms Druid exactly as before; this fix must not have
    /// silenced class evidence generally, only the teleport family
    #[test]
    fn an_ordinary_spell_of_the_same_class_still_confirms_it() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Tue Jul 28 15:01:00 2026] You have entered Blackburrow.",
            b"[Tue Jul 28 15:01:01 2026] You begin casting Cascade of Hail.",
            b"[Tue Jul 28 15:02:00 2026] You have entered West Karana.",
            b"[Tue Jul 28 15:02:01 2026] You begin casting Cascade of Hail.",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let you = ing.store.names.get("You").expect("You should be interned");
        let configured = ing
            .classes
            .configuration_of_visit(you.0, ing.zone.index_at(ing.now_ms()));
        assert!(configured.contains(&"Druid".to_string()), "{configured:?}");
    }

    /// why: real bug, caught live (user's own domain knowledge) --
    /// "Illusion: Dark Elf" (treated as rock-solid Enchanter-exclusive
    /// evidence) is also a click effect on "Guise of the Deceiver"/"Mask
    /// of Deception" -- the log line is identical whether it's a real
    /// class cast or an item click, so it must feed classdetect nothing.
    #[test]
    fn a_spell_with_a_known_item_click_source_contributes_no_class_evidence() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Tue Jul 28 15:01:00 2026] You have entered Blackburrow.",
            b"[Tue Jul 28 15:01:01 2026] You begin casting Illusion: Dark Elf.",
            b"[Tue Jul 28 15:02:00 2026] You have entered West Karana.",
            b"[Tue Jul 28 15:02:01 2026] You begin casting Illusion: Dark Elf.",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let you = ing.store.names.get("You").expect("You should be interned");
        assert!(
            ing.classes.configurations_of(you.0).is_empty(),
            "a spell with a known item source, cast twice, must still confirm nothing"
        );
    }

    /// why: real bug, caught live -- a pet's own cast (merged onto the
    /// owner's Sym for DPS-attribution purposes, correctly) still fed
    /// classdetect as if the owner had cast it themselves. Wrong even
    /// when the merge itself is accurate: a pet's ability kit doesn't
    /// prove which class its owner currently has active.
    #[test]
    fn a_pets_own_cast_contributes_no_class_evidence_for_the_owner() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Tue Jul 28 15:01:00 2026] You have entered Blackburrow.",
            b"[Tue Jul 28 15:01:01 2026] You summon forth a lesser familiar.",
            // why: a pet's first logged cast is its own spawn self-buff --
            // pairs it to the pending summon above (see note_actor's own doc)
            b"[Tue Jul 28 15:01:02 2026] Nifty begins casting Inner Fire.",
            b"[Tue Jul 28 15:01:03 2026] Nifty begins casting Cascade of Hail.",
            b"[Tue Jul 28 15:02:00 2026] You have entered West Karana.",
            b"[Tue Jul 28 15:02:01 2026] You summon forth a lesser familiar.",
            b"[Tue Jul 28 15:02:02 2026] Nifty begins casting Inner Fire.",
            b"[Tue Jul 28 15:02:03 2026] Nifty begins casting Cascade of Hail.",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let you = ing.store.names.get("You").expect("You should be interned");
        assert!(
            ing.classes.configurations_of(you.0).is_empty(),
            "the pet's own Druid-exclusive cast, twice, must still confirm nothing for the owner"
        );
    }
}

#[cfg(test)]
mod skill_evidence_tests {
    use super::*;
    use crate::parser::build_engine;

    /// why: skill.up was parsed but never routed -- confirms it now
    /// reaches classdetect like an unambiguous spell (Tracking is
    /// Bard/Druid/Ranger only). Elimination narrowing needs 3 distinct
    /// visits to corroborate, a stricter bar than an unambiguous cast's
    /// own 2 (real bug found live -- see classdetect module's own doc
    /// on MIN_ELIMINATION_CASTS), so the narrowing sequence repeats on
    /// 2 more visits here.
    #[test]
    fn a_tracking_skill_up_narrows_the_open_slot() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        // why: Enchanter/Wizard reinforced first; then an ambiguous pool
        // sharing only Ranger with Tracking's pool, repeated on 2 more
        // distinct visits (Highkeep, Runnyeye) -- elimination evidence
        // needs 3 distinct visits to corroborate, a stricter bar than an
        // unambiguous cast's own 2 (see MIN_ELIMINATION_CASTS's own doc)
        let lines: Vec<&[u8]> = vec![
            b"[Tue Jul 28 15:01:00 2026] You have entered Befallen.",
            b"[Tue Jul 28 15:01:01 2026] You begin casting Kilan's Animation.",
            b"[Tue Jul 28 15:01:02 2026] You begin casting Shock of Lightning.",
            b"[Tue Jul 28 15:02:00 2026] You have entered West Karana.",
            b"[Tue Jul 28 15:02:01 2026] You begin casting Kilan's Animation.",
            b"[Tue Jul 28 15:02:02 2026] You begin casting Shock of Lightning.",
            b"[Tue Jul 28 15:03:00 2026] You have entered Blackburrow.",
            b"[Tue Jul 28 15:03:01 2026] You begin casting Kilan's Animation.",
            b"[Tue Jul 28 15:03:02 2026] You begin casting Shock of Lightning.",
            // why: Endure Fire pool {Beastlord,Cleric,Druid,Ranger,Shaman} --
            // no item click source to muddy it, unlike Cure Poison
            b"[Tue Jul 28 15:03:03 2026] You begin casting Endure Fire.",
            // why: evasive stance narrows with Endure Fire to {Beastlord, Ranger}
            b"[Tue Jul 28 15:03:04 2026] You assume an evasive stance.",
            // why: Tracking {Bard,Druid,Ranger} -- only Ranger survives all
            // three pools, but only on this one visit so far -- not proof yet
            b"[Tue Jul 28 15:03:05 2026] You have become better at Tracking! (1)",
            b"[Tue Jul 28 15:04:00 2026] You have entered Highkeep.",
            b"[Tue Jul 28 15:04:01 2026] You begin casting Kilan's Animation.",
            b"[Tue Jul 28 15:04:02 2026] You begin casting Shock of Lightning.",
            b"[Tue Jul 28 15:04:03 2026] You begin casting Endure Fire.",
            b"[Tue Jul 28 15:04:04 2026] You assume an evasive stance.",
            b"[Tue Jul 28 15:04:05 2026] You have become better at Tracking! (2)",
            b"[Tue Jul 28 15:05:00 2026] You have entered Runnyeye.",
            b"[Tue Jul 28 15:05:01 2026] You begin casting Kilan's Animation.",
            b"[Tue Jul 28 15:05:02 2026] You begin casting Shock of Lightning.",
            b"[Tue Jul 28 15:05:03 2026] You begin casting Endure Fire.",
            b"[Tue Jul 28 15:05:04 2026] You assume an evasive stance.",
            b"[Tue Jul 28 15:05:05 2026] You have become better at Tracking! (3)",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let you = ing.store.names.get("You").expect("You should be interned");
        let configured = ing
            .classes
            .configuration_of_visit(you.0, ing.zone.index_at(ing.now_ms()));
        assert_eq!(
            configured,
            vec![
                "Enchanter".to_string(),
                "Ranger".to_string(),
                "Wizard".to_string()
            ],
            "{configured:?}"
        );
    }
}

#[cfg(test)]
mod invocation_evidence_tests {
    use super::*;
    use crate::parser::build_engine;

    /// why: invocation + stance + skill-up, none alone enough, narrow the
    /// open slot together like three ambiguous spells would.
    /// Elimination narrowing needs 3 distinct visits to corroborate, a
    /// stricter bar than an unambiguous cast's own 2 (real bug found
    /// live -- see classdetect module's own doc on MIN_ELIMINATION_CASTS),
    /// so the narrowing sequence repeats on 2 more visits here.
    #[test]
    fn an_invocation_combines_with_a_stance_and_a_skill_to_narrow_the_open_slot() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Tue Jul 28 15:01:00 2026] You have entered Befallen.",
            b"[Tue Jul 28 15:01:01 2026] You begin casting Kilan's Animation.",
            b"[Tue Jul 28 15:01:02 2026] You begin casting Shock of Lightning.",
            b"[Tue Jul 28 15:02:00 2026] You have entered West Karana.",
            b"[Tue Jul 28 15:02:01 2026] You begin casting Kilan's Animation.",
            b"[Tue Jul 28 15:02:02 2026] You begin casting Shock of Lightning.",
            b"[Tue Jul 28 15:03:00 2026] You have entered Blackburrow.",
            b"[Tue Jul 28 15:03:01 2026] You begin casting Kilan's Animation.",
            b"[Tue Jul 28 15:03:02 2026] You begin casting Shock of Lightning.",
            // why: Spellblade pool {Beastlord,Paladin,Ranger,Shadow Knight}
            b"[Tue Jul 28 15:03:03 2026] You begin reciting the spellblade invocation.",
            // why: Evasive narrows with Spellblade to {Beastlord, Ranger}
            b"[Tue Jul 28 15:03:04 2026] You assume an evasive stance.",
            // why: Tracking {Bard,Druid,Ranger} -- only Ranger survives all
            // three, but only on this one visit so far -- not proof yet
            b"[Tue Jul 28 15:03:05 2026] You have become better at Tracking! (1)",
            b"[Tue Jul 28 15:04:00 2026] You have entered Highkeep.",
            b"[Tue Jul 28 15:04:01 2026] You begin casting Kilan's Animation.",
            b"[Tue Jul 28 15:04:02 2026] You begin casting Shock of Lightning.",
            b"[Tue Jul 28 15:04:03 2026] You begin reciting the spellblade invocation.",
            b"[Tue Jul 28 15:04:04 2026] You assume an evasive stance.",
            b"[Tue Jul 28 15:04:05 2026] You have become better at Tracking! (2)",
            b"[Tue Jul 28 15:05:00 2026] You have entered Runnyeye.",
            b"[Tue Jul 28 15:05:01 2026] You begin casting Kilan's Animation.",
            b"[Tue Jul 28 15:05:02 2026] You begin casting Shock of Lightning.",
            b"[Tue Jul 28 15:05:03 2026] You begin reciting the spellblade invocation.",
            b"[Tue Jul 28 15:05:04 2026] You assume an evasive stance.",
            b"[Tue Jul 28 15:05:05 2026] You have become better at Tracking! (3)",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let you = ing.store.names.get("You").expect("You should be interned");
        let configured = ing
            .classes
            .configuration_of_visit(you.0, ing.zone.index_at(ing.now_ms()));
        assert_eq!(
            configured,
            vec![
                "Enchanter".to_string(),
                "Ranger".to_string(),
                "Wizard".to_string()
            ],
            "{configured:?}"
        );
    }
}

#[cfg(test)]
mod aa_evidence_tests {
    use super::*;
    use crate::parser::build_engine;

    /// why: real, curated class data (aadata.rs's own `category` field)
    /// was never wired into classdetect before -- Monk/Rogue have no
    /// exclusive spell/skill evidence anywhere else in the app, so a
    /// real AA grant is genuine new signal for them specifically
    #[test]
    fn a_known_class_aa_feeds_classdetect_like_an_unambiguous_spell() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Tue Jul 28 15:01:00 2026] You have entered Blackburrow.",
            b"[Fri Aug 07 00:25:51 2026] You have gained the ability \"Innate Sneakiness\" at a cost of 0 ability points.",
            b"[Tue Jul 28 15:02:00 2026] You have entered West Karana.",
            b"[Fri Aug 07 00:25:51 2026] You have gained the ability \"Innate Sneakiness\" at a cost of 0 ability points.",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let you = ing.store.names.get("You").expect("You should be interned");
        let configured = ing
            .classes
            .configuration_of_visit(you.0, ing.zone.index_at(ing.now_ms()));
        assert!(configured.contains(&"Rogue".to_string()), "{configured:?}");
    }

    /// why: an unrecognized AA name must contribute nothing, same as any other unmapped spell
    #[test]
    fn an_unrecognized_aa_contributes_no_class_evidence() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![b"[Fri Aug 07 00:25:51 2026] You have gained the ability \"Not A Real Ability\" at a cost of 0 ability points."];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let you = ing.store.names.get("You").expect("You should be interned");
        assert!(ing.classes.configurations_of(you.0).is_empty());
    }
}

#[cfg(test)]
mod effect_ping_tests {
    use super::*;
    use crate::parser::build_engine;

    /// why: a recognized buff-landing line pings state on "You" whether
    /// or not a Quick Buff window is open (an ally could have cast it)
    #[test]
    fn a_flavor_line_pings_state_with_no_quickbuff_window_open_at_all() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> =
            vec![b"[Sat Aug 08 00:01:02 2026] A burst of strength surges through your body."];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let you = ing.store.names.get("You").expect("You should be interned");
        let recent = ing.effects.recent_text(you.0, ing.now_ms(), 1_000);
        assert_eq!(
            recent,
            vec!["A burst of strength surges through your body."]
        );
    }

    /// why: recognizing state doesn't imply it's safe class evidence --
    /// pings fire across two visits with no Quick Buff window, nothing confirmed
    #[test]
    fn a_flavor_line_with_no_open_window_never_becomes_class_evidence() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Tue Jul 28 15:01:00 2026] You have entered Befallen.",
            b"[Tue Jul 28 15:01:01 2026] A blast of acid eats at your skin.",
            b"[Tue Jul 28 15:02:00 2026] You have entered West Karana.",
            b"[Tue Jul 28 15:02:01 2026] A blast of acid eats at your skin.",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let you = ing.store.names.get("You").expect("You should be interned");
        let configured = ing
            .classes
            .configuration_of_visit(you.0, ing.zone.index_at(ing.now_ms()));
        assert!(configured.is_empty(), "{configured:?}");

        // why: the ping itself still landed both times
        assert_eq!(
            ing.effects.recent_text(you.0, ing.now_ms(), 1_000),
            vec!["A blast of acid eats at your skin."]
        );
    }

    /// why: same text, same visits, but inside an open Quick Buff window
    /// each time -- two distinct-visit sightings confirm the class
    #[test]
    fn a_flavor_line_inside_an_open_quickbuff_window_still_confirms_a_class() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Tue Jul 28 15:01:00 2026] You have entered Befallen.",
            b"[Tue Jul 28 15:01:01 2026] You activate Quick Buff.",
            b"[Tue Jul 28 15:01:02 2026] A blast of acid eats at your skin.",
            b"[Tue Jul 28 15:02:00 2026] You have entered West Karana.",
            b"[Tue Jul 28 15:02:01 2026] You activate Quick Buff.",
            b"[Tue Jul 28 15:02:02 2026] A blast of acid eats at your skin.",
            // why: past PULSE_WINDOW_MS, flushes pending evidence; a
            // different flavor line on purpose -- same text would look like the pulsing pattern itself
            b"[Tue Jul 28 15:02:20 2026] A burst of strength surges through your body.",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let you = ing.store.names.get("You").expect("You should be interned");
        let configured = ing
            .classes
            .configuration_of_visit(you.0, ing.zone.index_at(ing.now_ms()));
        assert!(
            configured.contains(&"Necromancer".to_string()),
            "{configured:?}"
        );
    }

    /// why: end-to-end through a real encounter -- an ally's buff lands
    /// on "You" mid-fight, a scrub query shortly after shows it as recent
    #[test]
    fn fight_state_at_surfaces_a_recent_effect_on_you() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Tue Jul 28 15:02:11 2026] You hit Refugee Splitpaw for 10 points of damage.",
            b"[Tue Jul 28 15:02:12 2026] A burst of strength surges through your body.",
            b"[Tue Jul 28 15:02:13 2026] You hit Refugee Splitpaw for 12 points of damage.",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let enc_id = ing
            .enc_map
            .values()
            .next()
            .copied()
            .expect("one encounter should be open")
            .0;
        let states = crate::combat::fight_state_at(&ing, enc_id, ing.now_ms());
        let you_state = states
            .iter()
            .find(|s| s.name == "You")
            .expect("You should be in the fight");
        let texts: Vec<&str> = you_state
            .recent_effects
            .iter()
            .map(|e| e.text.as_str())
            .collect();
        assert_eq!(texts, vec!["A burst of strength surges through your body."]);
    }

    /// why: a third-person landing line pings state on whoever it landed
    /// on, not "You"; real bard-song pulse cadence from the reference log
    #[test]
    fn a_third_person_landing_pings_state_on_the_actual_target_not_you() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Fri Aug 07 16:30:31 2026] Handstuff's voice booms.",
            b"[Fri Aug 07 16:30:37 2026] Handstuff's voice booms.",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let handstuff = ing
            .store
            .names
            .get("Handstuff")
            .expect("Handstuff should be interned");
        assert_eq!(
            ing.effects.recent_text(handstuff.0, ing.now_ms(), 60_000),
            vec!["Your voice booms.", "Your voice booms."],
            "canonical first-person text, not the raw third-person line"
        );

        let you = ing.store.names.get("You");
        assert!(
            you.is_none_or(|s| ing
                .effects
                .recent_text(s.0, ing.now_ms(), 60_000)
                .is_empty()),
            "must not land on You -- it landed on Handstuff"
        );
    }

    /// why: never class evidence even inside an open Quick Buff window --
    /// a third-person line doesn't prove an ally cast it
    #[test]
    fn a_third_person_landing_never_becomes_class_evidence() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Tue Jul 28 15:01:00 2026] You have entered Befallen.",
            b"[Tue Jul 28 15:01:01 2026] You activate Quick Buff.",
            b"[Tue Jul 28 15:01:02 2026] Handstuff's voice booms.",
            b"[Tue Jul 28 15:02:00 2026] You have entered West Karana.",
            b"[Tue Jul 28 15:02:01 2026] You activate Quick Buff.",
            b"[Tue Jul 28 15:02:02 2026] Handstuff's voice booms.",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);

        // why: "You" never gets interned here -- if evidence leaked through, known_entities would show it
        assert!(
            ing.classes.known_entities().next().is_none(),
            "no evidence should ever be attributed to anyone from these lines"
        );
    }

    /// why: a possessive line whose reconstruction isn't a known message
    /// pings nothing -- the dictionary gate stops false positives
    #[test]
    fn a_possessive_line_that_does_not_reconstruct_a_known_message_pings_nothing() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Fri Aug 07 16:30:31 2026] Bravesirrobin's hand is covered with a nonexistent aura.",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let bravesirrobin = ing.store.names.get("Bravesirrobin");
        assert!(bravesirrobin.is_none_or(|s| ing
            .effects
            .recent_text(s.0, ing.now_ms(), 60_000)
            .is_empty()));
    }

    /// why: conjugated (non-possessive) third-person form, from the user's own report
    #[test]
    fn a_conjugated_third_person_landing_pings_state_on_the_actual_target() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![b"[Fri Aug 07 16:30:31 2026] Draxiz N`Ryt feels much faster."];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let draxiz = ing
            .store
            .names
            .get("Draxiz N`Ryt")
            .expect("Draxiz N`Ryt should be interned");
        assert_eq!(
            ing.effects.recent_text(draxiz.0, ing.now_ms(), 60_000),
            vec!["You feel much faster."]
        );
    }

    /// why: multi-word name proves the split-point search works
    /// regardless of name length, plus the irregular "are" -> "is" conjugation
    #[test]
    fn a_multi_word_name_still_resolves_the_correct_split_point() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Fri Aug 07 22:03:04 2026] The Prophet is struck by a sudden force.",
            b"[Fri Aug 07 22:03:09 2026] The Prophet is struck by a sudden burst of force.",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let prophet = ing
            .store
            .names
            .get("The Prophet")
            .expect("The Prophet should be interned");
        assert_eq!(
            ing.effects.recent_text(prophet.0, ing.now_ms(), 60_000),
            vec![
                "You are struck by a sudden force.",
                "You are struck by a sudden burst of force.",
            ]
        );
    }

    /// why: "Your <noun> <verb>" recovery -- reconstructs "Your feet adhere...", not "You adhere..."
    #[test]
    fn a_your_noun_verb_shape_recovers_its_own_real_key() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![b"[Fri Aug 07 16:30:31 2026] Akkirus adheres to the ground."];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let akkirus = ing
            .store
            .names
            .get("Akkirus")
            .expect("Akkirus should be interned");
        assert_eq!(
            ing.effects.recent_text(akkirus.0, ing.now_ms(), 60_000),
            vec!["Your feet adhere to the ground."]
        );
    }

    /// why: "@ combusts." is the one confirmed-by-hand alias -- a
    /// shortened announcement, not a conjugated one, per the user's correction
    #[test]
    fn the_named_combust_alias_pings_the_canonical_first_person_text() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![b"[Fri Aug 07 16:30:31 2026] Baron Telyx V`Zher combusts."];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let baron = ing
            .store
            .names
            .get("Baron Telyx V`Zher")
            .expect("Baron Telyx V`Zher should be interned");
        assert_eq!(
            ing.effects.recent_text(baron.0, ing.now_ms(), 60_000),
            vec!["You feel your skin combust."]
        );
    }

    /// why: used to be a genuine gap ("no first-person text exists for
    /// this at all, pings nothing rather than guessing") -- closed by
    /// spelltext::match_effect_polarity: the tail is shared by 4 real
    /// SoW-family spells (Pack Spirit/Spirit of Bih`Li/Spirit of Scale/
    /// Spirit of Wolf), all buffs, so it's a confident polarity ping
    /// even with no single confident spell name to attach.
    #[test]
    fn ambiguous_third_person_landing_text_still_pings_a_polarity() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> =
            vec![b"[Fri Aug 07 16:30:31 2026] Lenekab is surrounded by a brief lupine aura."];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let lenekab = ing.store.names.get("Lenekab").expect("Lenekab interned");
        let recent = ing.effects.recent_text(lenekab.0, ing.now_ms(), 60_000);
        assert_eq!(
            recent,
            vec!["Lenekab is surrounded by a brief lupine aura."]
        );
    }

    /// why: noun-keeping sibling of plain verb-conjugation -- same shape
    /// as combust above, but this one keeps the noun in third person
    #[test]
    fn a_feel_your_noun_verb_line_keeps_the_noun_in_third_person() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> =
            vec![b"[Fri Aug 07 16:30:31 2026] orc legionnaire's body pulses with energy."];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let orc = ing
            .store
            .names
            .get("orc legionnaire")
            .expect("orc legionnaire should be interned");
        assert_eq!(
            ing.effects.recent_text(orc.0, ing.now_ms(), 60_000),
            vec!["You feel your body pulse with energy."]
        );
    }

    /// why: trailing "you." -> "them." -- pronoun swaps too, not just the subject
    #[test]
    fn a_trailing_you_swaps_to_them_when_it_lands_on_someone_else() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> =
            vec![b"[Fri Aug 07 16:30:31 2026] Dreadmoon feels the favor of the gods upon them."];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let dreadmoon = ing
            .store
            .names
            .get("Dreadmoon")
            .expect("Dreadmoon should be interned");
        assert_eq!(
            ing.effects.recent_text(dreadmoon.0, ing.now_ms(), 60_000),
            vec!["You feel the favor of the gods upon you."]
        );
    }

    /// why: "feel ADJ" -> "looks ADJ" family -- renders as visible appearance, not conjugated "feels ADJ."
    #[test]
    fn a_single_adjective_buff_recognizes_its_looks_form() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![b"[Fri Aug 07 16:30:31 2026] Draxiz N`Ryt looks dexterous."];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let draxiz = ing
            .store
            .names
            .get("Draxiz N`Ryt")
            .expect("Draxiz N`Ryt should be interned");
        assert_eq!(
            ing.effects.recent_text(draxiz.0, ing.now_ms(), 60_000),
            vec!["You feel dexterous."]
        );
    }

    /// why: cast.blocked's spell name is definite class evidence,
    /// proven via two zone visits, same path a landed cast would use
    #[test]
    fn a_blocked_cast_still_confirms_class_evidence_from_its_own_spell_name() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Tue Jul 28 15:01:00 2026] You have entered Befallen.",
            b"[Fri Aug 14 21:11:25 2026] Your Color Flux spell did not take hold on Hakujin. (Blocked by Berserker Spirit.)",
            b"[Tue Jul 28 15:02:00 2026] You have entered West Karana.",
            b"[Fri Aug 14 21:11:25 2026] Your Color Flux spell did not take hold on Joneker. (Blocked by Berserker Spirit.)",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);

        // why: Color Flux -- Enchanter-exclusive, confirmed against spell_classes.json, no item click source to muddy the evidence
        let you = ing.store.names.get("You").expect("You should be interned");
        let configured = ing
            .classes
            .configuration_of_visit(you.0, ing.zone.index_at(ing.now_ms()));
        assert!(
            configured.contains(&"Enchanter".to_string()),
            "{configured:?}"
        );
    }

    /// why: blocker half -- names a buff already active on the target, fed to Effects as usual
    #[test]
    fn a_blocked_cast_pings_the_blocker_as_state_on_the_target() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Fri Aug 14 21:11:25 2026] Your Boon of the Clear Mind spell did not take hold on Joneker. (Blocked by Clarity.)",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let joneker = ing
            .store
            .names
            .get("Joneker")
            .expect("Joneker should be interned");
        assert_eq!(
            ing.effects.recent_text(joneker.0, ing.now_ms(), 60_000),
            vec!["Clarity"]
        );
    }

    /// why: real minority with no trailing parenthetical -- class evidence still lands, nothing to ping
    #[test]
    fn a_blocked_cast_with_no_named_blocker_still_confirms_class_evidence() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Tue Jul 28 15:01:00 2026] You have entered Befallen.",
            b"[Sat Aug 15 09:49:02 2026] Your Shield of Lava spell did not take hold on Bravesirrobin.",
            b"[Tue Jul 28 15:02:00 2026] You have entered West Karana.",
            b"[Sat Aug 15 09:49:02 2026] Your Shield of Lava spell did not take hold on Kaeus.",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let bravesirrobin = ing.store.names.get("Bravesirrobin");
        assert!(bravesirrobin.is_none_or(|s| ing
            .effects
            .recent_text(s.0, ing.now_ms(), 60_000)
            .is_empty()));
    }

    /// why: dot.damage_from_you previously fell through both existing
    /// damage-from rules ("damage from your ..." matched neither); now a real Damage row
    #[test]
    fn a_dot_tick_credited_via_your_is_a_real_damage_row() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Fri Aug 07 00:00:07 2026] Orc scoutsman has taken 104 damage from your Elemental Maelstrom X.",
            b"[Sun Aug 09 04:21:39 2026] Orc scoutsman has taken 16 damage from your Elemental Maelstrom X. (Critical)",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let you = ing.store.names.get("You").expect("You should be interned");
        let rows = by_ability(&ing.store, &Filter::default().by(you));
        let row = rows
            .iter()
            .find(|r| ing.store.ability_name(r.ability) == "Elemental Maelstrom X")
            .expect("an Elemental Maelstrom X row should exist");
        assert_eq!(row.total, 120);
        assert_eq!(row.hits, 2);
        assert_eq!(row.crits, 1);
    }

    /// why: poison/disease matched but produced no Action before
    /// StateEffect existed to feed -- now real pings, self and third-party
    #[test]
    fn poison_and_disease_now_ping_state_on_the_right_entity() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Fri Aug 07 19:52:47 2026] You have been diseased.",
            b"[Fri Aug 07 19:52:48 2026] Dojii has been diseased.",
            b"[Fri Aug 07 19:52:49 2026] You have been poisoned.",
            b"[Fri Aug 07 19:52:50 2026] a rattlesnake has been poisoned.",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let you = ing.store.names.get("You").expect("You should be interned");
        assert_eq!(
            ing.effects.recent_text(you.0, ing.now_ms(), 60_000),
            vec!["Diseased", "Poisoned"]
        );
        let dojii = ing
            .store
            .names
            .get("Dojii")
            .expect("Dojii should be interned");
        assert_eq!(
            ing.effects.recent_text(dojii.0, ing.now_ms(), 60_000),
            vec!["Diseased"]
        );
        let snake = ing
            .store
            .names
            .get("a rattlesnake")
            .expect("rattlesnake should be interned");
        assert_eq!(
            ing.effects.recent_text(snake.0, ing.now_ms(), 60_000),
            vec!["Poisoned"]
        );
    }

    /// why: named yaulp alias -- same effect as scraped text, reworded not conjugated
    #[test]
    fn the_named_yaulp_alias_pings_the_canonical_first_person_text() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> =
            vec![b"[Fri Aug 07 16:33:09 2026] Flewdur lets loose a mighty yaulp."];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let flewdur = ing
            .store
            .names
            .get("Flewdur")
            .expect("Flewdur should be interned");
        assert_eq!(
            ing.effects.recent_text(flewdur.0, ing.now_ms(), 60_000),
            vec!["You feel a surge of strength as you let forth a mighty yaulp."]
        );
    }

    /// why: "feel X" -> "is X" family (multi-word tail), a second substitute-verb pattern off "feel"
    #[test]
    fn a_feel_x_buff_recognizes_its_is_x_form() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> =
            vec![b"[Fri Aug 07 16:30:31 2026] Bravesirrobin is protected from magic."];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let bravesirrobin = ing
            .store
            .names
            .get("Bravesirrobin")
            .expect("Bravesirrobin should be interned");
        assert_eq!(
            ing.effects
                .recent_text(bravesirrobin.0, ing.now_ms(), 60_000),
            vec!["You feel protected from magic."]
        );
    }

    /// why: ability.activated is class evidence for the activator, not
    /// "You" -- two visits confirm Rogue for the activator, not the log owner
    #[test]
    fn an_activated_ability_confirms_class_evidence_for_its_own_activator() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Tue Jul 28 15:01:00 2026] You have entered Befallen.",
            b"[Thu Aug 13 21:46:48 2026] Aella activates Asp Venom.",
            b"[Thu Aug 13 21:46:57 2026] Aella activates Antimagic Poison.",
            b"[Tue Jul 28 15:02:00 2026] You have entered West Karana.",
            b"[Thu Aug 13 21:47:05 2026] Aella activates Antimagic Poison.",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let aella = ing
            .store
            .names
            .get("Aella")
            .expect("Aella should be interned");
        let configured = ing
            .classes
            .configuration_of_visit(aella.0, ing.zone.index_at(ing.now_ms()));
        assert!(configured.contains(&"Rogue".to_string()), "{configured:?}");

        // why: Aella herself, not "You" -- log owner gets no evidence from a line they weren't subject of
        assert!(
            ing.store.names.get("You").is_none(),
            "You should never be interned by this"
        );
    }

    /// why: state-ping half -- fed to Effects on the activator regardless of whether classdata recognizes it
    #[test]
    fn an_activated_ability_pings_state_on_its_activator_even_when_unrecognized() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![b"[Fri Aug 07 00:00:00 2026] Bigneum activates Skull Bash."];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let bigneum = ing
            .store
            .names
            .get("Bigneum")
            .expect("Bigneum should be interned");
        assert_eq!(
            ing.effects.recent_text(bigneum.0, ing.now_ms(), 60_000),
            vec!["Skull Bash"]
        );
    }

    /// why: regression guard -- the general rule must never shadow Quick Buff's own dedicated rule
    #[test]
    fn quick_buff_still_opens_its_own_window_not_the_general_rule() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Tue Jul 28 15:01:00 2026] You activate Quick Buff.",
            b"[Tue Jul 28 15:01:01 2026] A blast of acid eats at your skin.",
            b"[Tue Jul 28 15:02:00 2026] You have entered West Karana.",
            b"[Tue Jul 28 15:02:01 2026] You activate Quick Buff.",
            b"[Tue Jul 28 15:02:02 2026] A blast of acid eats at your skin.",
            // why: different flavor line past PULSE_WINDOW_MS, flushes pending evidence
            b"[Tue Jul 28 15:02:20 2026] A burst of strength surges through your body.",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let you = ing.store.names.get("You").expect("You should be interned");
        let configured = ing
            .classes
            .configuration_of_visit(you.0, ing.zone.index_at(ing.now_ms()));
        assert!(
            configured.contains(&"Necromancer".to_string()),
            "{configured:?}"
        );
    }

    /// why: real false positive the user caught -- a group-wide buff
    /// landing on the player and an ally within the activation window;
    /// "Magician" must never get confirmed, it was never the player's own Quick Buff
    #[test]
    fn a_group_cast_landing_on_someone_else_cancels_pending_quickbuff_evidence() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Tue Jul 28 15:01:00 2026] You have entered Befallen.",
            b"[Tue Jul 28 15:01:00 2026] You activate Quick Buff.",
            b"[Tue Jul 28 15:01:02 2026] You are enveloped by flame.",
            b"[Tue Jul 28 15:01:03 2026] Kabanab is enveloped by flame.",
            b"[Tue Jul 28 15:02:00 2026] You have entered West Karana.",
            b"[Tue Jul 28 15:02:00 2026] You activate Quick Buff.",
            b"[Tue Jul 28 15:02:02 2026] You are enveloped by flame.",
            b"[Tue Jul 28 15:02:03 2026] Kabanab is enveloped by flame.",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let you = ing.store.names.get("You").expect("You should be interned");
        let configured = ing
            .classes
            .configuration_of_visit(you.0, ing.zone.index_at(ing.now_ms()));
        assert!(
            !configured.contains(&"Magician".to_string()),
            "a group cast on Kabanab too must never confirm Magician for the player: {configured:?}"
        );

        // why: ping is still real and unconditional -- only the class attribution gets cancelled
        assert_eq!(
            ing.effects.recent_text(you.0, ing.now_ms(), 60_000),
            vec!["You are enveloped by flame."]
        );
    }

    /// why: positive control -- a genuine solo Quick Buff burst still
    /// confirms class evidence; the fix narrows false positives without breaking true ones
    #[test]
    fn a_solo_quickbuff_landing_with_no_group_cast_still_confirms_class_evidence() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Tue Jul 28 15:01:00 2026] You have entered Befallen.",
            b"[Tue Jul 28 15:01:00 2026] You activate Quick Buff.",
            b"[Tue Jul 28 15:01:02 2026] A blast of acid eats at your skin.",
            b"[Tue Jul 28 15:02:00 2026] You have entered West Karana.",
            b"[Tue Jul 28 15:02:00 2026] You activate Quick Buff.",
            b"[Tue Jul 28 15:02:02 2026] A blast of acid eats at your skin.",
            // why: different flavor line past PULSE_WINDOW_MS, flushes the second window's pending evidence
            b"[Tue Jul 28 15:02:20 2026] A burst of strength surges through your body.",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let you = ing.store.names.get("You").expect("You should be interned");
        let configured = ing
            .classes
            .configuration_of_visit(you.0, ing.zone.index_at(ing.now_ms()));
        assert!(
            configured.contains(&"Necromancer".to_string()),
            "{configured:?}"
        );
    }

    /// why: other false-positive shape -- a maintained ally buff pulsing
    /// only on the player; cross-entity check can't catch it, but its own repeat cadence gives it away
    #[test]
    fn a_pulsing_ally_buff_on_only_the_player_cancels_pending_quickbuff_evidence() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Tue Jul 28 15:01:00 2026] You have entered Befallen.",
            // why: pulsing before the player ever Quick Buffs -- proves it isn't tied to Quick Buff timing
            b"[Tue Jul 28 15:01:29 2026] You feel an aura of mystic protection surrounding you.",
            b"[Tue Jul 28 15:01:35 2026] You feel an aura of mystic protection surrounding you.",
            b"[Tue Jul 28 15:01:39 2026] You activate Quick Buff.",
            b"[Tue Jul 28 15:01:41 2026] You feel an aura of mystic protection surrounding you.",
            b"[Tue Jul 28 15:02:00 2026] You have entered West Karana.",
            b"[Tue Jul 28 15:02:23 2026] You feel an aura of mystic protection surrounding you.",
            b"[Tue Jul 28 15:02:29 2026] You feel an aura of mystic protection surrounding you.",
            b"[Tue Jul 28 15:02:33 2026] You activate Quick Buff.",
            b"[Tue Jul 28 15:02:35 2026] You feel an aura of mystic protection surrounding you.",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let you = ing.store.names.get("You").expect("You should be interned");
        let configured = ing
            .classes
            .configuration_of_visit(you.0, ing.zone.index_at(ing.now_ms()));
        assert!(
            !configured.contains(&"Bard".to_string()),
            "a maintained ally song must never confirm Bard for the player: {configured:?}"
        );

        // why: still real, unconditional state -- only class attribution is cancelled
        assert!(!ing
            .effects
            .recent_text(you.0, ing.now_ms(), 60_000)
            .is_empty());
    }

    /// why: real /loc line from the reference log
    #[test]
    fn a_loc_reading_is_captured_as_last_loc() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> =
            vec![b"[Tue Aug 18 22:28:36 2026] Your Location is 216.51, -103.09, -20.19"];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let (ts, x, y, z) = ing
            .last_loc
            .expect("a /loc reading should have been captured");
        assert_eq!(ts, ing.now_ms());
        assert_eq!(x, 216.51);
        assert_eq!(y, -103.09);
        assert_eq!(z, -20.19);
    }

    /// why: real cast->zone.enter sequence (~15s apart) marks the visit
    /// a confirmed Wizard teleport, with wiki-sourced coordinates
    #[test]
    fn a_translocate_cast_followed_by_zoning_marks_the_visit_teleported() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Sat Aug 01 13:19:46 2026] You begin casting Translocate: North Karana.",
            b"[Sat Aug 01 13:20:01 2026] You have entered The Northern Plains of Karana.",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);
        let (_, landing) = ing
            .entered_via_teleport
            .expect("should be marked teleported");
        assert_eq!(landing.class, teleportdata::TeleportClass::Wizard);
        assert_eq!((landing.x, landing.y, landing.z), (-3685.0, 1209.0, -5.0));
    }

    /// why: Circle of X is a Druid teleport, distinguished from Wizard's Translocate
    #[test]
    fn a_circle_cast_followed_by_zoning_marks_the_visit_druid_teleported() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Sat Aug 01 13:19:46 2026] You begin casting Circle of Karana.",
            b"[Sat Aug 01 13:20:01 2026] You have entered The Northern Plains of Karana.",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);
        let (_, landing) = ing
            .entered_via_teleport
            .expect("should be marked teleported");
        assert_eq!(landing.class, teleportdata::TeleportClass::Druid);
        assert_eq!((landing.x, landing.y, landing.z), (-2706.0, -1494.0, -4.0));
    }

    /// why: a proven ally's teleport cast counts too -- group-shaped
    /// spells land the whole group, so an ally caster still marks your visit teleported
    #[test]
    fn a_proven_allys_translocate_cast_marks_your_visit_teleported() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Sat Aug 01 13:19:00 2026] Dippinsauce tells the group, 'incoming port'",
            b"[Sat Aug 01 13:19:46 2026] Dippinsauce begins casting Translocate: North Karana.",
            b"[Sat Aug 01 13:20:01 2026] You have entered The Northern Plains of Karana.",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);
        let (_, landing) = ing
            .entered_via_teleport
            .expect("should be marked teleported");
        assert_eq!(landing.class, teleportdata::TeleportClass::Wizard);
    }

    /// why: an ordinary zone-line walk with no recent teleport cast must not read as a landing
    #[test]
    fn an_ordinary_zone_change_is_not_marked_teleported() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![b"[Tue Jul 28 15:01:00 2026] You have entered Blackburrow."];
        backfill_lines(&mut ing, &engine, &lines, 1);
        assert!(ing.entered_via_teleport.is_none());
    }

    /// why: a teleport cast too long before the zone change must not be
    /// credited -- else an unrelated later zone-line walk wrongly reads as a landing
    #[test]
    fn a_stale_translocate_cast_does_not_mark_a_later_zone_change() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Sat Aug 01 13:19:46 2026] You begin casting Translocate: North Karana.",
            b"[Sat Aug 01 14:00:00 2026] You have entered The Northern Plains of Karana.",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);
        assert!(ing.entered_via_teleport.is_none());
    }

    /// why: bare "Gate" (returns to bind point, not a fixed landmark)
    /// has no coordinate data and must never trigger the spire guess
    #[test]
    fn a_bare_gate_cast_does_not_mark_the_visit_teleported() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Thu Jul 30 17:27:54 2026] You begin casting Gate.",
            b"[Thu Jul 30 17:28:05 2026] You have entered The Feerrott.",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);
        assert!(ing.entered_via_teleport.is_none());
    }

    /// why: `<Zone> Gate` names a real landmark with a wiki-confirmed
    /// landing and does count as a Wizard teleport
    #[test]
    fn a_named_gate_cast_marks_the_visit_wizard_teleported() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Thu Jul 30 17:27:54 2026] You begin casting Cazic Temple Gate.",
            b"[Thu Jul 30 17:28:05 2026] You have entered Cazic Thule.",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);
        let (_, landing) = ing
            .entered_via_teleport
            .expect("should be marked teleported");
        assert_eq!(landing.class, teleportdata::TeleportClass::Wizard);
    }

    /// why: "Circle of X" that's actually a resist buff, not a teleport
    /// -- real false positive the old name-shape-only heuristic had
    #[test]
    fn a_name_shape_false_positive_does_not_mark_the_visit_teleported() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Thu Jul 30 17:27:54 2026] You begin casting Circle of Summer.",
            b"[Thu Jul 30 17:28:05 2026] You have entered Blackburrow.",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);
        assert!(ing.entered_via_teleport.is_none());
    }

    /// why: an unproven stranger's teleport cast must never mark "You" -- only You or a proven ally counts
    #[test]
    fn another_players_translocate_does_not_mark_your_visit_teleported() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Sat Aug 01 13:19:46 2026] Dippinsauce begins casting Translocate: North Karana.",
            b"[Sat Aug 01 13:20:01 2026] You have entered The Northern Plains of Karana.",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);
        assert!(ing.entered_via_teleport.is_none());
    }

    /// why: Origin has no fixed wiki destination (starting-city AA) so
    /// it can't use the teleport table -- learned_origin learns it from a real cast+zone.enter pair
    #[test]
    fn an_origin_cast_followed_by_zoning_learns_the_landing_zone() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Tue Jul 28 15:51:23 2026] You begin casting Origin.",
            b"[Tue Jul 28 15:51:46 2026] You have entered Oggok.",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);
        let (_, zone) = ing
            .learned_origin
            .expect("should have learned an origin zone");
        assert_eq!(zone, "Oggok");
        // why: Origin has no fixed coordinate, unlike Gate/Translocate -- stays None
        assert!(ing.entered_via_teleport.is_none());
    }

    /// why: real Origin landing changed over time -- learned_origin must
    /// track the most recent confirmation, "last one wins" like the other fields
    #[test]
    fn a_later_origin_confirmation_overwrites_an_earlier_one() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Tue Jul 28 15:51:23 2026] You begin casting Origin.",
            b"[Tue Jul 28 15:51:46 2026] You have entered Oggok.",
            b"[Wed Jul 29 17:00:03 2026] You begin casting Origin.",
            b"[Wed Jul 29 17:00:18 2026] You have entered Neriak - Commons.",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);
        let (_, zone) = ing
            .learned_origin
            .expect("should have learned an origin zone");
        assert_eq!(zone, "Neriak - Commons");
    }

    /// why: interrupted cast with no retry and no zone change learns
    /// nothing; known gap -- an unrelated zone walk within the window would still be wrongly learned
    #[test]
    fn an_interrupted_cast_alone_learns_nothing() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Tue Jul 28 16:23:38 2026] You begin casting Origin.",
            b"[Tue Jul 28 16:23:53 2026] Your Origin spell is interrupted.",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);
        assert!(ing.learned_origin.is_none());
    }

    /// why: "last one wins" self-heal -- an interrupted cast retried and
    /// landing learns the retry's own zone; real shape in the reference log
    #[test]
    fn a_fizzled_cast_immediately_retried_learns_the_retrys_own_zone() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Tue Jul 28 16:23:38 2026] You begin casting Origin.",
            b"[Tue Jul 28 16:23:53 2026] Your Origin spell is interrupted.",
            b"[Tue Jul 28 16:28:05 2026] You begin casting Origin.",
            b"[Tue Jul 28 16:28:20 2026] You have entered Oggok.",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);
        let (_, zone) = ing
            .learned_origin
            .expect("the retry should have learned a zone");
        assert_eq!(zone, "Oggok");
    }

    /// why: real bug -- "Ruins of Old Guk" and "gukbottom" share no
    /// text, so a substring guess can't confirm it; zone_matches must
    /// resolve to "Lower Guk" so who_name -> map_shortnames reaches "gukbottom"
    #[test]
    fn a_real_zone_with_no_textual_resemblance_to_its_map_shortname_still_resolves() {
        let raw = "The Ruins of Old Guk 4 (Refined)";
        let z = crate::zonedata::zones()
            .iter()
            .find(|z| crate::zone::zone_matches(raw, &z.name))
            .expect("should resolve to a real wiki zone");
        assert_eq!(z.name, "Lower Guk");

        let who_name = z
            .who_name
            .as_deref()
            .expect("Lower Guk should carry a real who_name in the bundled scrape");
        assert!(
            crate::zonedata::map_shortnames(who_name).contains(&"gukbottom".to_string()),
            "who_name {who_name:?} should resolve to the real map file shortname"
        );
    }

    /// why: real spell, real unique landing text (confirmed against
    /// packs/spells.json: "You are pelted by hailstones." names only
    /// Cascade of Hail, catalog-wide) -- proves attribution reaches past
    /// the trivial self-cast case: the landing text itself never says who
    /// cast it, only real-cast-timing correlation can.
    #[test]
    fn a_third_partys_recent_cast_is_attributed_as_the_source_of_a_self_landing_effect() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Tue Jul 28 15:02:11 2026] Dippinsauce begins casting Cascade of Hail.",
            // why: real casting_time is 2.75s -- 3s later lands well inside ATTRIBUTION_TOLERANCE_MS
            b"[Tue Jul 28 15:02:14 2026] You are pelted by hailstones.",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let you = ing.store.names.get("You").expect("You should be interned");
        let recent = ing.effects.recent(you.0, ing.now_ms(), 10_000);
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].source.as_deref(), Some("Dippinsauce"));
        assert_eq!(recent[0].skill.as_deref(), Some("Cascade of Hail"));
    }

    /// why: real spells, real shared landing text -- "You feel much
    /// better." names 8 different real spells catalog-wide (confirmed),
    /// so spelltext::match_spell_text must drop it as ambiguous. Only
    /// "Healing" was actually cast nearby here, so attribute_effect's own
    /// local disambiguation (checking real recent casts, not the whole
    /// catalog) must still resolve it confidently.
    #[test]
    fn globally_ambiguous_text_resolves_locally_when_only_one_real_recent_cast_explains_it() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Tue Jul 28 15:02:11 2026] Dippinsauce begins casting Healing.",
            // why: real casting_time is 2.5s
            b"[Tue Jul 28 15:02:14 2026] You feel much better.",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let you = ing.store.names.get("You").expect("You should be interned");
        let recent = ing.effects.recent(you.0, ing.now_ms(), 10_000);
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].source.as_deref(), Some("Dippinsauce"));
        assert_eq!(recent[0].skill.as_deref(), Some("Healing"));
    }

    /// why: real bug shape to guard against -- two different real
    /// entities cast two different real spells sharing the same globally
    /// ambiguous text, both close enough in time to explain the same
    /// landing. Neither source nor skill has one confident answer here;
    /// must stay honestly unresolved, not guess either candidate.
    #[test]
    fn two_real_simultaneous_candidates_leave_source_and_skill_unresolved() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Tue Jul 28 15:02:11 2026] Dippinsauce begins casting Healing.",
            b"[Tue Jul 28 15:02:12 2026] Bravesirrobin begins casting Greater Healing.",
            b"[Tue Jul 28 15:02:14 2026] You feel much better.",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let you = ing.store.names.get("You").expect("You should be interned");
        let recent = ing.effects.recent(you.0, ing.now_ms(), 10_000);
        assert_eq!(recent.len(), 1);
        assert_eq!(
            recent[0].source, None,
            "2 real candidates -- no confident source"
        );
        assert_eq!(
            recent[0].skill, None,
            "2 real candidates -- no confident skill either"
        );
    }

    /// why: real spell whose own name already appears verbatim as the
    /// ping text (Action::AbilityActivated's own shape, "X activates
    /// Y." -- ability = the real spell name directly, no flavor sentence
    /// involved at all) -- attribute_effect's own tier 1
    #[test]
    fn a_ping_that_is_already_a_real_spell_name_still_attributes_its_caster() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Tue Jul 28 15:02:11 2026] Dippinsauce begins casting Antimagic Poison.",
            b"[Tue Jul 28 15:02:11 2026] Dippinsauce activates Antimagic Poison.",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let dippinsauce = ing
            .store
            .names
            .get("Dippinsauce")
            .expect("Dippinsauce should be interned");
        let recent = ing.effects.recent(dippinsauce.0, ing.now_ms(), 10_000);
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].skill.as_deref(), Some("Antimagic Poison"));
        assert_eq!(recent[0].source.as_deref(), Some("Dippinsauce"));
    }
}
