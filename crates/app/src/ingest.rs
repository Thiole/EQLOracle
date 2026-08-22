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

/// How many distinct unmatched-line "shapes" (`eqlp_core::shape` --
/// variable text collapsed into a stable template) the Debug module's
/// coverage tab keeps before new ones stop being tracked. Reuses the same
/// bound the `eqlp coverage` CLI command tunes for the identical purpose
/// -- this is the same clustering, just accumulated live instead of run
/// offline after the fact. A shape already being tracked keeps counting
/// past the cap; only a brand-new one gets dropped, folded into
/// `Ingest::unmatched_shapes_overflow` instead.
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

/// Feed rows kept between drains -- bounds memory if the UI is slow to
/// drain a burst. The frontend keeps its own smaller rolling window on top.
const MAX_PENDING_RECENT: usize = 500;

/// How long a "<Owner> summons a <flavour>." line stays a candidate for
/// matching against the next new "Inner Fire" caster. The log never names
/// the pet, only the owner and flavour -- attribution comes from a pet's
/// spawn behaviour (several pet flavours reliably self-buff with Inner
/// Fire within a couple of seconds of being summoned) rather than from any
/// direct textual link, and only that specific spell counts as a
/// candidate signal -- see `Ingest::note_actor` for why "any first cast"
/// was tried and found unsafe against real data. 8s was chosen by
/// measuring against the 2M-line reference log: matching each summon
/// against the closest new Inner-Fire caster within the window resolved
/// dozens of real pets with no implausible pairing in a manual spot check.
const PET_MATCH_WINDOW_MS: Millis = 8_000;

/// How long after "<Name> activates Quick Buff." an unmatched line still
/// counts as one of that activation's own buffs landing -- see
/// `Ingest::flavor_evidence_for`. Chosen generously against real examples
/// in the reference log, where a full quickbuff burst's landing lines all
/// resolve within the same or the next couple of log-seconds after the
/// activation line.
const QUICKBUFF_WINDOW_MS: Millis = 5_000;

/// How close together two entities' landings of the *same* recognized
/// text have to be before that's treated as proof of a group cast rather
/// than coincidence -- see `Ingest::attribute_flavor_hit`'s doc for why
/// this exists at all. Confirmed directly against a real false positive
/// in the reference log: a group-wide buff landed on the player and three
/// named allies within the same log-second, 3 seconds after the player's
/// own Quick Buff activation -- well inside `QUICKBUFF_WINDOW_MS`, so it
/// would otherwise have been credited as the player's own buff.
const GROUP_CAST_WINDOW_MS: Millis = 3_000;

/// How long after a recognized teleport cast (`teleportdata::landing_for`)
/// a subsequent `zone.enter` still counts as that cast's own landing, for
/// the Maps module's entrance guess. Generous on purpose: cast time plus
/// the "LOADING, PLEASE WAIT..." screen: confirmed in the reference log at
/// ~15s total (13:19:46 cast start -> 13:20:01 zone.enter) for
/// Translocate specifically, and interrupted/cancelled Gate casts
/// (confirmed real: 39 in the reference log) never produce a zone.enter
/// at all, so a short-ish window costs nothing there -- this gives
/// headroom without risking crediting an unrelated later zone change.
const TELEPORT_WINDOW_MS: Millis = 30_000;

/// How recently the *same* text has to have already landed on the *same*
/// entity before a fresh landing is treated as an externally-maintained
/// buff pulsing, rather than a one-shot Quick Buff proc -- the other real
/// false-positive shape found in the reference log, and the dominant one:
/// a single-target ally song re-cast repeatedly on the player, which
/// never lands on anyone else at all (so `GROUP_CAST_WINDOW_MS`'s
/// cross-entity check can't see it), but has a real, distinct temporal
/// signature Quick Buff doesn't -- Quick Buff applies once at activation
/// time and does not recur; a maintained song does, on a short, regular
/// cadence. Measured directly against real repeats in the log ("You feel
/// an aura of mystic protection surrounding you." pulsing at 17:07:23,
/// :29, :35, :41, :47, :53 -- a steady ~6s cadence sustained for minutes),
/// this window only needs to span one or two pulses to catch it.
const PULSE_WINDOW_MS: Millis = 15_000;

/// Safety-net threshold for `Store::close_stale_encounters` -- far looser
/// than the graph layer's own 10s idle close, on purpose: this only exists
/// to catch what slips past that normal path, not to replace it.
const STALE_ENCOUNTER_MS: Millis = 5 * 60 * 1000;

/// The player's own effective (account) level over time, from `level.up`
/// lines ("You have gained a level! Welcome to level N!") -- first-person
/// only, so this only ever tracks "You", never an ally.
///
/// This is the *effective* level, not any one class's own -- and the two
/// genuinely diverge in this game. Swapping one class in a 3-class loadout
/// for a lower-level one drops the effective level to match the new
/// lowest class, with no log line marking the drop itself (only the climb
/// back up re-fires `level.up` lines, for levels already passed once
/// before) -- confirmed directly against a real log: level climbs 2->50
/// over five real days, then a swap visibly drops it (`level.up` lines
/// resume from 14, not 51, immediately after a config change), climbs
/// back to 36, drops again to 11. A single "current level" number would
/// misrepresent this -- see `Millis`-keyed `at` below, which answers "what
/// was the effective level as of this specific instant" so a caller can
/// build a level range for a whole configuration out of its own member
/// visits/fights rather than one point-in-time snapshot standing in for
/// all of them.
#[derive(Debug, Clone, Default)]
pub struct Levels {
    /// Every `level.up` line, in log-time order (the order they arrive in
    /// a forward tail/backfill replay) -- `observe` trusts this and does
    /// not re-sort, matching every other timestamped structure in this
    /// file.
    at_ts: Vec<(Millis, u8)>,
}

/// One AA rank purchase, in the order the log reports it.
#[derive(Debug, Clone)]
pub struct AaGrant {
    pub name: String,
    /// 1 for `aa.gained` (first purchase -- the log's own "gained the
    /// ability" line never states a rank number, that line *is* rank 1),
    /// the parsed rank for `aa.improved` (rank 2+).
    pub rank: u8,
    /// Ability points spent on this one rank -- 0 for a free first rank,
    /// which several real AAs have.
    pub cost: u32,
}

/// Every AA rank purchase seen this session ("You have gained the ability
/// ...\"/"You have improved ..." lines), in log-time order -- an
/// append-only log with no interpretation, the same shape `Levels` above
/// uses for `level.up`. Catalog enrichment (category/description) is a
/// separate lookup, `crate::aadata::aa_by_name`, kept apart from this raw
/// record the same way `zone.rs`'s raw label and `zonedata`'s wiki match
/// stay separate.
#[derive(Debug, Clone, Default)]
pub struct AaLog {
    at_ts: Vec<(Millis, AaGrant)>,
}

impl AaLog {
    pub fn observe(&mut self, ts: Millis, name: String, rank: u8, cost: u32) {
        self.at_ts.push((ts, AaGrant { name, rank, cost }));
    }

    /// Every grant, in log-time order.
    pub fn all(&self) -> impl Iterator<Item = &(Millis, AaGrant)> {
        self.at_ts.iter()
    }

    /// Total ability points spent across every rank purchase seen this
    /// session -- a free rank contributes 0, same as the log itself says.
    pub fn total_spent(&self) -> u32 {
        self.at_ts.iter().map(|(_, g)| g.cost).sum()
    }
}

/// Highest live in-game rank observed cast this session, for "You"
/// specifically, keyed by catalog base spell name -- e.g. "You begin
/// casting Ice Comet X." confirms rank 10 of `Ice Comet`. Real, and
/// common: 2,131 "You begin casting Ice Comet X." lines alone in the
/// reference log, rank visibly climbing over real time (IV early, X
/// later) -- confirmed directly, not assumed. This is a *third* thing,
/// distinct from `packs/spells.json`'s own name (which never carries
/// this -- see `split_cast_rank`'s own doc) and distinct from
/// `base_spell_name`'s cast-line rank-stripping (which discards the
/// number entirely, only caring whether two cast/damage lines name the
/// same spell). Session-only like every other `Ingest`-owned log here --
/// there's no server-side or log-side record of "current rank" to
/// recover on restart beyond replaying casts.
#[derive(Debug, Clone, Default)]
pub struct SpellRanks {
    best: HashMap<String, (u8, Millis)>,
}

impl SpellRanks {
    /// Only keeps a rank if it's the highest seen so far for that spell --
    /// a real rank is never supposed to regress, but treating a same-or-
    /// lower re-observation as a no-op is simpler than asserting that and
    /// just as correct in practice.
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

    /// Every spell with an observed rank this session, unordered.
    pub fn all(&self) -> impl Iterator<Item = (&str, u8)> {
        self.best.iter().map(|(k, (r, _))| (k.as_str(), *r))
    }
}

/// Every "Your <item> (Exaltation) <flavor text>." line seen this session
/// (`proc.item`'s rule in `packs/eql.toml`, filtered to the `Exaltation`
/// effect label -- see `extract_action`'s own `"proc.item"` arm), keyed
/// by item name.
///
/// This exists because of a real, hard data gap: neither the `/outputfile
/// inventory` dump (confirmed against a real one: `Location`, `Name`,
/// `ID`, `Count`, `Slots` -- an augment-socket *count*, nothing about
/// exaltation) nor any other log line ever reveals what's actually
/// socketed into a given item's exaltation slots. A combat proc firing is
/// the *only* thing this app can ever confirm about them -- that the
/// item's own Proc socket is genuinely live, not what spell effect
/// resulted (an earlier attempt at inferring that from adjacent log
/// lines turned out to be statistically meaningless: 85% of *all* casts
/// in a real log are immediately preceded by *some* Exaltation shimmer
/// line, since gear procs constantly during combat -- see git history/
/// session notes on the Harm Touch investigation for the full story).
/// So this only ever answers "has this item's proc fired, and how many
/// times", never "with what effect".
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

    /// `0` for an item that's never fired its proc this session -- same
    /// as genuinely not having one, which is the honest reading: this app
    /// has no way to tell "no proc" from "a proc that hasn't fired yet".
    pub fn count(&self, item: &str) -> u32 {
        self.counts.get(item).copied().unwrap_or(0)
    }

    pub fn first_seen_ms(&self, item: &str) -> Option<Millis> {
        self.first_seen_ms.get(item).copied()
    }
}

/// Two-tier confidence for a spell's spellbook membership, from EQL's own
/// begin/finish line pairs -- real for *both* scribing a new scroll
/// ("Beginning to scribe X..." / "You have finished scribing X.", 596/593
/// occurrences in the reference log -- the actual "added to spellbook"
/// event, superseding an earlier version of this module that assumed no
/// such line existed) and memorizing a gem ("Beginning to memorize X..."
/// / "You have finished memorizing X.", which a spell can only complete
/// if it's already known, so it's equally good proof).
///
/// **Known**: a "finished" line landed at least once, scribe or
/// memorize, either is definitive. **Possible**: a "Beginning to..." line
/// landed but no matching "finished" ever followed for that spell,
/// either direction -- real reasons this happens: an interrupted/failed
/// attempt (593 of 596 real scribes completed, so a few genuinely don't),
/// or the log simply ends mid-action. Once a spell reaches Known it stays
/// there permanently -- a later interrupted re-memorize of an
/// already-confirmed spell doesn't downgrade it back to Possible.
///
/// Deduped to first-seen rather than a full history (unlike `AaLog`)
/// because a single spell can complete memorization hundreds of times in
/// one session as gems get swapped between fights -- the interesting
/// fact is "is this known, or just possible" and "when did we first see
/// evidence", not every individual re-memorize.
#[derive(Debug, Clone, Default)]
pub struct SpellLog {
    entries: HashMap<String, SpellEvidence>,
}

#[derive(Debug, Clone, Copy)]
struct SpellEvidence {
    /// First "Beginning to..." (scribe or memorize) ever seen for this
    /// spell -- kept even after `finished` is set, so `first_seen`
    /// doesn't jump forward once a spell graduates to Known.
    first_began: Millis,
    /// First "finished" (scribe or memorize) ever seen, if any --
    /// `Some` is what makes a spell Known rather than merely Possible.
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

    /// Every spell that reached Known -- name, and when it was first
    /// confirmed -- arbitrary order (caller sorts as needed).
    pub fn known(&self) -> impl Iterator<Item = (&str, Millis)> {
        self.entries
            .iter()
            .filter_map(|(k, e)| e.finished.map(|ts| (k.as_str(), ts)))
    }

    /// Every spell that's only reached Possible -- a "Beginning to..."
    /// line with no matching "finished" ever seen for it. Name, and when
    /// the attempt began.
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

    /// Every `level.up` value whose own line's timestamp falls in
    /// `[start, end)` -- `end: None` means unbounded above (the visit is
    /// still open). Deliberately does *not* reach outside the window for
    /// a stale value carried over from whatever configuration was active
    /// right before `start`, the way `at(start)` would -- see this
    /// struct's own doc for why that distinction is the whole point here:
    /// a config-swap drop is real and silent, so treating "whatever the
    /// tracker last said" as this configuration's own evidence mixes in
    /// a different configuration's level entirely.
    pub fn between(&self, start: Millis, end: Option<Millis>) -> impl Iterator<Item = u8> + '_ {
        let from = self.at_ts.partition_point(|&(t, _)| t < start);
        let to = match end {
            Some(e) => self.at_ts.partition_point(|&(t, _)| t < e),
            None => self.at_ts.len(),
        };
        self.at_ts[from..to].iter().map(|&(_, l)| l)
    }

    /// The effective level as of `ts` -- the most recent `level.up` at or
    /// before it, or `None` before the first one ever seen (level 1, which
    /// never gets its own line since nobody starts below it).
    pub fn at(&self, ts: Millis) -> Option<u8> {
        let i = self.at_ts.partition_point(|&(t, _)| t <= ts);
        if i == 0 {
            None
        } else {
            Some(self.at_ts[i - 1].1)
        }
    }

    /// The most recently observed level, full stop -- `at` needs a
    /// timestamp to answer "as of when", this is just "as of now". `None`
    /// under the same condition `at` is: no `level.up` line has been seen
    /// in this file's history at all, which in practice mostly means
    /// "you've been this level for the whole time this log file covers" --
    /// a ding fires the line, sitting at a level doesn't. Callers that want
    /// a level for something (`gearplanner`'s mana weighting) need to
    /// treat that as "unknown", not "level 1".
    pub fn latest(&self) -> Option<u8> {
        self.at_ts.last().map(|&(_, l)| l)
    }

    /// The timestamp of that same most-recently-observed `level.up` line --
    /// `latest`'s own "as of when", for a caller (`overview::session`)
    /// that needs to know how long the *current* level has been active,
    /// not just what it is, to measure XP progress within it rather than
    /// across however much of the file happens to be in view.
    pub fn latest_ts(&self) -> Option<Millis> {
        self.at_ts.last().map(|&(t, _)| t)
    }
}

/// One recognized buff/effect landing on "You", captured verbatim from its
/// own landing-message text (`crate::flavordata`) -- there is no companion
/// "wears off" line for these in this game's log (checked directly against
/// the reference log: only poison logs its own end), so this is a *ping*,
/// not an interval. It says an effect landed at `ts`; it says nothing about
/// how long it lasted.
#[derive(Debug, Clone)]
pub struct EffectPing {
    pub ts: Millis,
    pub text: String,
}

/// Append-only per-entity log of recognized effect landings -- deliberately
/// separate from `eqlp_session::timeline::Timeline`, whose `State` is a
/// single exclusive value (engaged/mezzed/...). Buffs stack: several can be
/// live on the same entity at once, so there's no one "current state" to
/// query, only "what landed recently" -- see `recent`. In practice this is
/// only ever populated for "You": `flavordata`'s dictionary keys are the
/// scraped landing text verbatim, which is always written in first-person
/// ("Your ...", "You feel ..."), so a third-person line naming some other
/// entity never matches it at all -- no separate filtering needed to keep
/// this scoped to the player.
#[derive(Debug, Clone, Default)]
pub struct Effects {
    by_entity: HashMap<u32, Vec<EffectPing>>,
}

impl Effects {
    /// Inserted in timestamp order rather than blind-pushed, same safety
    /// `Timeline::push` uses -- a late-arriving line (out-of-order chunk
    /// merge) must not corrupt the ordering `recent`'s binary search
    /// depends on.
    fn push(&mut self, entity: u32, ts: Millis, text: String) {
        let v = self.by_entity.entry(entity).or_default();
        let at = v.partition_point(|p| p.ts <= ts);
        v.insert(at, EffectPing { ts, text });
    }

    /// Effect text that landed on `entity` within `window_ms` up to and
    /// including `ts` -- the same trailing-window snapshot idea
    /// `dps_window` uses for "what's happening right now", not a claim
    /// about whether the effect is still active (this data can't say that
    /// -- see `EffectPing`'s doc).
    pub fn recent(&self, entity: u32, ts: Millis, window_ms: Millis) -> Vec<&str> {
        let Some(v) = self.by_entity.get(&entity) else {
            return Vec::new();
        };
        let from = ts - window_ms;
        let a = v.partition_point(|p| p.ts < from);
        let b = v.partition_point(|p| p.ts <= ts);
        v[a..b].iter().map(|p| p.text.as_str()).collect()
    }

    /// Every ping ever recorded for `entity`, oldest first -- mirrors
    /// `eqlp_session::timeline::Timeline::transitions_of`. For a caller
    /// that wants the whole history rather than one instant's trailing
    /// window (e.g. a future full buff log view).
    pub fn all(&self, entity: u32) -> &[EffectPing] {
        self.by_entity.get(&entity).map(|v| v.as_slice()).unwrap_or(&[])
    }
}

/// Recovers a third-person buff/effect landing's first-person dictionary
/// key, if `text` is shaped like one and the recovery actually lands on a
/// known key -- `"<Name>'s <tail>."` becomes `"Your <tail>."`, the same
/// text `flavordata`'s scraper always records (first/second-person only,
/// see its module doc). Returns `(who, canonical_text)`.
///
/// A single mechanical transform, not per-spell rules -- and it's the
/// dictionary lookup that keeps it safe: checked directly against the
/// reference log's own unmatched backlog (76,986 real possessive-shaped
/// lines), this recovers 35 distinct real spell families (buffs and DoT
/// ticks alike -- "Deathklokk's voice booms." -> Bard's Amplification
/// landing on someone else's target; "a willowisp's skin blisters and
/// burns." -> a caster's DoT ticking on a mob) with no false positives
/// possible -- a possessive sentence that isn't really one of these (an
/// item name, "granted spell" line, "capturing X's attention") just fails
/// to reconstruct a real key and is correctly ignored.
///
/// Only the *first* `"'s "` in the line is treated as the possessive
/// boundary. This game's own stylized names use a backtick (`` O`Keil ``),
/// not an apostrophe, for exactly this reason -- confirmed against real
/// misses in the same audit (`"O\`Keil's Radiation"` splits cleanly on the
/// straight apostrophe, landing on the *spell* name, not the player's).
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

/// English 3rd-person-singular-present conjugation of `verb`, for
/// reconstructing a third-person landing line's own first-person
/// dictionary key -- e.g. "feels" is this game's own third-person text
/// for "you feel"; see `verb_suffix_table`. Only the two truly irregular
/// verbs actually seen in `spell_flavor.json` ("are"/"have") are
/// special-cased; everything else is the regular English rule. Deliberately
/// not a general-purpose conjugator -- a bad guess here just fails a
/// dictionary lookup downstream (see `verb_conjugated_flavor`), so
/// correctness only matters for the finite, already-scraped verb set this
/// runs against, not arbitrary English.
fn conjugate_third_person(verb: &str) -> Option<String> {
    if !verb.bytes().all(|b| b.is_ascii_lowercase()) {
        return None;
    }
    Some(match verb {
        "are" => "is".to_string(),
        "have" => "has".to_string(),
        v if v.ends_with(['s', 'x', 'z']) || v.ends_with("sh") || v.ends_with("ch") || v.ends_with('o') => {
            format!("{v}es")
        }
        v if v.ends_with('y') && v.len() > 1 && !matches!(v.as_bytes()[v.len() - 2], b'a' | b'e' | b'i' | b'o' | b'u') => {
            format!("{}ies", &v[..v.len() - 1])
        }
        v => format!("{v}s"),
    })
}

/// A handful of third-person landings whose relationship to their
/// first-person dictionary key genuinely isn't a grammatical transform --
/// the game shortens the sentence itself, not just its conjugation --
/// confirmed individually rather than derived. `"combusts."` is the
/// concrete case: `"You feel your skin combust."` is the full first-person
/// flavor, but the third-person announcement drops "feel your skin"
/// entirely (`"Orc slaver combusts."`, not `"...'s skin combusts."`,
/// unlike `ignite`/`freeze`/`pulse`'s siblings below, which *do* keep the
/// noun -- there's no single rule covering both, so this one stays a
/// named exception rather than forcing a rule to fit one data point).
const THIRD_PERSON_VERB_ALIASES: &[(&str, &str)] = &[
    ("combusts.", "You feel your skin combust."),
    // "lets loose a mighty yaulp." (517 real occurrences) -- same Yaulp
    // effect as "You feel a surge of strength as you let forth a mighty
    // yaulp.", just reworded ("let forth" -> "lets loose") rather than
    // conjugated.
    ("lets loose a mighty yaulp.", "You feel a surge of strength as you let forth a mighty yaulp."),
];

/// Third-person-suffix -> canonical first-person `spell_flavor.json` key,
/// built once from the dictionary itself. Every rule here started as a
/// hypothesis checked against the reference log's real unmatched backlog
/// before being trusted -- see each rule's own note for the real numbers.
///
/// - `"You <verb> <tail>."` -> `"<verb+s> <tail>."` (`"You feel much
///   faster."` -> `"feels much faster."`, matching real lines like
///   `"Draxiz N\`Ryt feels much faster."`; 79 distinct real hits).
/// - `"Your <noun> <verb> <tail>."` -> `"<verb+s> <tail>."` (only one real
///   spell uses this shape -- `"Your feet adhere to the ground."` ->
///   `"adheres to the ground."`).
/// - `"You feel your <noun> <verb>[ <tail>]."` -> `"'s <noun> <verb+s>[
///   <tail>]."` -- the third person keeps the possessed noun instead of
///   dropping it (contrast `THIRD_PERSON_VERB_ALIASES`'s `combust`, which
///   doesn't): `"You feel your body pulse with energy."` ->
///   `"'s body pulses with energy."`, matching `"orc legionnaire's body
///   pulses with energy."` (4 distinct real hits, thousands of lines --
///   `ignite`/`freeze`/`freeze over`/`pulse with energy`).
/// - A trailing `" you."` in an already-built suffix also gets a `"
///   them."` sibling -- when the effect lands on someone else, the
///   sentence's own trailing pronoun swaps too, not just the subject
///   (`"You convulse as the lightning arcs through you."` ->
///   `"convulses as the lightning arcs through them."`, matching real
///   lines; 3 distinct real hits).
/// - `"You feel <word>."` (a single-word adjective, nothing else) also
///   gets a `"looks <word>."` sibling, alongside the regular `"feels
///   <word>."` one -- this game renders a whole family of single-
///   adjective buffs as how the *target* visibly looks to onlookers, not
///   just how they feel (`"You feel dexterous."` -> `"looks dexterous."`,
///   matching `"Draxiz N\`Ryt looks dexterous."`; 14 distinct real hits
///   across agile/charismatic/daring/dexterous/frail/healthy/nimble/
///   protected/resolute/robust/strong/stronger/valorous/weaker).
/// - `"You feel <tail>."` (any length this time, not just one word) also
///   gets an `"is <tail>."` sibling -- a second, separate substitute-verb
///   family off the same source (`"You feel protected from magic."` ->
///   `"is protected from magic."`, matching `"Bravesirrobin is protected
///   from magic."`; 8 distinct real hits, dominated by the
///   protected-from-{magic,disease,poison,cold,fire} family).
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
                    // "You feel <tail>." -> also try "looks <tail>."
                    // (single-word adjectives only) and "is <tail>." (any
                    // length) -- two separate real substitute-verb
                    // families off the same "feel" source, not one rule --
                    // see this fn's own doc.
                    if verb == "feel" {
                        if !tail.contains(' ') {
                            table.insert(format!("looks {tail}"), key);
                        }
                        table.insert(format!("is {tail}"), key);
                    }
                }
            }
            // "You feel your <noun> <verb>[ <tail>]." -> "'s <noun>
            // <verb+s>[ <tail>]." -- the noun-keeping sibling of the
            // plain "You <verb> <tail>." rule above.
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
            let (Some(_noun), Some(verb), Some(tail)) = (parts.next(), parts.next(), parts.next()) else {
                continue;
            };
            if let Some(conj) = conjugate_third_person(verb) {
                table.insert(format!("{conj} {tail}"), key);
            }
        }
        // Trailing " you." -> " them." sibling for every suffix built so
        // far -- collected separately and inserted after, so this pass
        // can't see (and re-derive from) its own output.
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

/// Recovers a third-person buff/effect landing's first-person dictionary
/// key when the two forms differ only by ordinary verb conjugation, not a
/// bare possessive (`third_person_flavor` already covers `"<Name>'s
/// <first-person-text-verbatim>"`). Two kinds of split point, both tried
/// at every occurrence rather than just the first (an entity name can be
/// multiple words -- `"Baron Telyx V\`Zher combusts."` -- so the split
/// point isn't knowable in advance):
///
/// - Plain space: `"<Name> <verb> ..."` against `verb_suffix_table`'s
///   `"You <verb> ..."`-derived entries (`"Draxiz N\`Ryt feels much
///   faster."`).
/// - `"'s "` (glued to the name, not space-delimited, so it needs its own
///   scan): `"<Name>'s <noun> <verb> ..."` against the table's
///   noun-keeping entries, which are stored *with* their leading `"'s "`
///   -- see `verb_suffix_table`'s own doc (`"orc legionnaire's body
///   pulses with energy."`).
///
/// Safe for the same reason `third_person_flavor` is: a wrong split just
/// fails the table lookup, so this can't manufacture a match out of an
/// unrelated sentence.
///
/// Checked directly against the reference log before being trusted:
/// reconstructing every dictionary entry's own conjugated suffix and
/// searching for it for real recovers 79 distinct spell families, tens of
/// thousands of real lines (`"feels much better."` alone: 6,080) --
/// broader than `third_person_flavor`'s own 35. Also confirms the
/// negative: `"adheres to the ground."`'s sibling candidates that *don't*
/// recover a real key (most of the 361+102 candidates tried) correctly
/// find nothing, rather than a false positive -- the dictionary gate is
/// what makes trying every split point safe.
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

/// Quick Buff class evidence waiting out its cancellation window -- see
/// `Ingest::attribute_flavor_hit`'s doc.
struct PendingQuickbuffEvidence {
    ts: Millis,
    who: u32,
    classes: &'static [String],
    text: String,
}

/// One not-yet-attributed `EventKind::Xp` row -- see `Ingest::pending_xp`'s
/// doc for why this needs to exist at all instead of a search like loot's.
struct PendingXp {
    /// Index into every `Store` column -- `self.store.enc[row]` is what
    /// gets backfilled once (if) a matching death resolves this.
    row: u32,
    ts: Millis,
}

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
    /// Raw zone label (interned `Sym`) -> the wiki zone it resolves to,
    /// if any -- resolved once per *distinct* label ever seen this
    /// session (`resolved_wiki_zone`, called from `current_zone` the
    /// moment a fresh one is stamped onto an `Encounter`), not re-run
    /// per encounter or per query. A session might see a couple hundred
    /// distinct zone labels at most; a fight count in the thousands all
    /// sharing a handful of those labels is the case this actually saves
    /// work on -- see `resolved_wiki_zone`'s own doc.
    wiki_zone_cache: HashMap<Sym, Option<&'static str>>,
    /// Per-mob-name (lowercased, so casing differences between two lines
    /// naming the same mob don't split them into separate buckets) --
    /// "which encounter am I currently looting". No timestamp alongside
    /// it (an earlier version paired this with "when was the last loot
    /// line" instead) -- see `recent_encounter_for`'s doc for exactly how
    /// this and `loot_claimed` combine to match loot lines to kill order
    /// rather than just picking whichever same-named encounter is most
    /// recent, and why validity is judged off the *encounter's own*
    /// activity instead.
    loot_cursor: HashMap<String, EncounterId>,
    /// Every encounter that's already had at least one loot line
    /// attributed to it -- lets `recent_encounter_for` skip a corpse
    /// that's already claimed and correctly advance to the *next*
    /// same-named one instead of reusing the first one forever.
    loot_claimed: HashSet<EncounterId>,
    /// The most recently pushed, not-yet-attributed `EventKind::Xp` row --
    /// its store index plus its own timestamp. See `record_xp`'s doc for
    /// why "You gain experience!" needs this instead of a `recent_
    /// encounter_for`-style search: unlike loot, it's *emitted before* the
    /// death line it belongs to, not after, so there's nothing to search
    /// for yet at the moment it's parsed. `record_death` is what resolves
    /// this, checking it against the death's own `ts`. Holds at most one
    /// entry: a fresh gain always overwrites whatever was here, since only
    /// the newest can still be waiting on its own matching death.
    pending_xp: Option<PendingXp>,
    /// The very first timestamp `apply` has ever processed -- "freshly
    /// logged on" for `session_start`'s purposes, the fallback for a
    /// player who's never gone AFK anywhere in this file. Set once, on the
    /// first line, and never touched again.
    first_ts: Option<Millis>,
    /// Whether the player is AFK as of the most recently processed line --
    /// `afk.on`/`afk.off`. Not itself used by `session_start` (going AFK
    /// doesn't retroactively shrink the *current* session, only *ending*
    /// one does), but surfaced by `currently_afk` for the Overview tab's
    /// own status display.
    afk_state: bool,
    /// Timestamp of the most recent `afk.off` line -- `session_start`'s
    /// preferred answer over `first_ts` when present. See that method's
    /// own doc for why a return from AFK reads as a fresh session rather
    /// than picking back up an old one.
    last_afk_off: Option<Millis>,
    /// Entity states (mez/charm/dead) keyed by the same `Sym` the store
    /// uses -- see `docs/design/timeline.md`. Session-wide rather than one
    /// per encounter: `state_at`/`between` already take an explicit time
    /// range, so scoping to one fight is a query, not a second table.
    pub timeline: Timeline,
    /// Per-cast outcome: landed, resisted, interrupted, fizzled, or
    /// unconfirmed. Keyed on interned name `Sym`s reused from `store.names`
    /// -- see `eqlp_session::cast` for why, and for the rank-recovery
    /// caveat on `confirm_landed`.
    casts: CastResolver,
    /// Per-entity class evidence, grouped by zone visit and never reset --
    /// see `eqlp_session::classdetect`'s module doc for why grouping by
    /// visit (rather than one rolling combination) is what keeps an
    /// occasional loadout from being crowded out of the picture entirely.
    /// `pub` for the same reason `timeline` is: `combat.rs` reads it
    /// directly rather than through a wrapper method.
    pub classes: ClassDetector,
    /// The player's own effective (account) level over time, from
    /// `level.up` lines only -- see `Levels`'s doc for what this can and
    /// can't say about any one class's own level.
    pub levels: Levels,
    /// Recognized buff/effect landings on "You" over time -- see
    /// `Effects`' own doc for why this is a separate log from `timeline`
    /// rather than folded into `State`.
    pub effects: Effects,
    /// The most recent `/loc` reading (`state.location`), if any --
    /// `(ts, x, y, z)`, in the coordinate order the line itself printed
    /// them. Frontend note, kept here since this field is the only real
    /// source of that data: `mapsdata.rs`'s map files do NOT use this same
    /// order -- confirmed (by brute-forcing every sign/order combination
    /// against 9 real readings in Lower Guk, scored by distance to that
    /// zone's own real wall geometry) that a map file's own (x, y) is
    /// this reading's `(-y, -x)`, z untouched. Two earlier guesses were
    /// each wrong in a different way and each looked plausible against a
    /// single sample: a supposed swap carried over from other EQ tooling
    /// conventions (never actually verified), and then "no swap at all"
    /// (which fit one sample by chance but averaged 80+ units off, with a
    /// 294-unit outlier, once checked against more). The real mapping
    /// averages 9.3 units off across all 9, max 16.1 -- see
    /// MapViewer.svelte, the only place this gets plotted, for the
    /// up-to-date working. Rare -- only set when the player types `/loc`
    /// -- so this is a "last known position" snapshot, not continuous
    /// tracking; a caller showing it should always display the timestamp
    /// alongside it.
    pub last_loc: Option<(Millis, f64, f64, f64)>,
    /// Timestamp and confirmed landing of "You" or a proven ally most
    /// recently beginning a recognized teleport cast
    /// (`teleportdata::landing_for`) -- e.g. a Wizard's `Translocate:
    /// <Zone>` (per the reference log's own confirmation line, "Do you
    /// wish to be translocated ... to <Zone>?"), which teleports the
    /// caster (and, for the group-shaped siblings, the whole group)
    /// straight to a fixed spot in the named zone, no travel. An ally's
    /// cast counts too, not just "You": a group-shaped Translocate/
    /// Portal/Circle/Ring is cast by one Wizard or Druid and lands
    /// everyone in the group, so gating on "You" alone would miss it
    /// whenever someone else in the group is the caster -- see `is_ally`
    /// for what "proven ally" means here (an unproven stranger's cast
    /// deliberately does not count, see
    /// `another_players_translocate_does_not_mark_your_visit_teleported`).
    /// Not scoped to a zone visit; only ever read within
    /// `TELEPORT_WINDOW_MS` of itself by `entered_via_teleport`.
    last_teleport_cast: Option<(Millis, teleportdata::TeleportLanding)>,
    /// The exact landing (if any) the *current* zone visit was entered
    /// via, rather than an ordinary zone-line walk -- set on every
    /// `Action::Zone`. The Maps module's entrance guess plots this exact
    /// coordinate directly (see `teleportdata`'s own doc for the
    /// coordinate-space caveat) instead of the weaker previous-zone-
    /// entrance guess. See `get_zone_context`.
    ///
    /// Timestamped (the confirming `zone.enter`'s own `ts`) -- a real,
    /// reported bug this fixes: `commands::live_start_position` used to
    /// pick a real `/loc` reading over this unconditionally, whenever its
    /// own zone matched, even when that `/loc` was typed *before* this
    /// teleport and is now stale -- a fresher, more certain landing
    /// losing to an older one just because `/loc` outranked it by kind,
    /// not by recency. Every consumer now compares this timestamp against
    /// `last_loc`'s own and takes whichever is actually newer.
    pub entered_via_teleport: Option<(Millis, teleportdata::TeleportLanding)>,
    /// Timestamp of "You" most recently beginning an `Origin` cast --
    /// personal only, unlike `last_teleport_cast`: the AA's own real
    /// description ("transports *you* back to your starting city",
    /// confirmed against `~/eql/aa.json`) is singular, not group-shaped
    /// like Translocate/Circle, so an ally's cast never counts here. See
    /// `learned_origin`'s own doc for why this exists as a second,
    /// parallel mechanism rather than folding into `last_teleport_cast`.
    last_origin_cast: Option<Millis>,
    /// Timestamp and raw zone label of the most recent real-world
    /// confirmation of where `Origin` actually sends this character.
    /// Origin, per the user's own direct point, is a genuinely *dynamic*
    /// teleport: unlike every spell `teleportdata::landing_for` covers,
    /// it has no single wiki-quotable destination at all (confirmed
    /// directly against the real reference log: this character's own
    /// Origin casts landed in four different real zones over three
    /// weeks -- Oggok, Neriak - Commons, New Sebilis Expedition, and The
    /// Feerrott -- settling into New Sebilis Expedition as the
    /// overwhelmingly dominant, current answer). So instead of a static
    /// lookup, this is *learned* empirically, the same "last one wins"
    /// shape `last_teleport_cast`/`entered_via_teleport` already use for
    /// a fizzle-then-retry: set on `Action::Zone` whenever it lands
    /// within `TELEPORT_WINDOW_MS` of `last_origin_cast`, overwritten by
    /// every later confirmation, self-correcting if the player ever
    /// changes their actual starting city again. Not itself a coordinate
    /// -- once a zone is known, "where in that zone" is exactly
    /// `routing::best_start_position`'s own question (the real,
    /// game-accurate succor point), computed lazily by whichever command
    /// needs it (`base_dir` isn't available at ingest time) rather than
    /// stored here.
    pub learned_origin: Option<(Millis, String)>,
    /// Every AA rank purchase seen this session -- `aa.gained`/`aa.
    /// improved` lines only. See `AaLog`'s own doc.
    pub aa: AaLog,
    /// Every spell confirmed known this session. See `SpellLog`'s own doc.
    pub spellbook: SpellLog,
    /// Highest live rank observed cast this session, "You" only. See
    /// `SpellRanks`' own doc.
    pub spell_ranks: SpellRanks,
    /// Every "Your <item> (Exaltation) ..." combat-proc line seen this
    /// session. See `ExaltationProcs`' own doc.
    pub exaltation_procs: ExaltationProcs,
    enc_map: HashMap<EncId, EncounterId>,
    /// Every entity seen in each store encounter so far, kept current as a
    /// fight grows (a multi-mob pull adds to it) rather than frozen at
    /// whichever mob was hit first -- `store::Encounter` only carries one
    /// label, but a fight can hold several entities. See `link`.
    pub entities_by_enc: HashMap<EncounterId, Vec<String>>,
    /// How far into `encounters.closed` we've synced to the store.
    /// `Builder` only ever appends to that vec, never drains it.
    closed_seen: usize,
    /// Unresolved "<Owner> summons a <flavour>." sightings, newest last,
    /// waiting to be matched against the next brand-new actor. Pruned to
    /// `PET_MATCH_WINDOW_MS` in `note_actor`.
    pending_summons: Vec<(Millis, String)>,
    /// Names ever seen acting (dealing damage, healing, missing, casting)
    /// -- an entity only gets checked against `pending_summons` the first
    /// time it acts, not on every subsequent action.
    seen_actors: HashSet<String>,
    /// Resolved pet -> owner, both already `display_name`-canonicalised.
    /// Checked by `sym` before interning, so a matched pet's every action
    /// -- including the one that triggered the match -- merges into the
    /// owner's identity rather than becoming its own entity. See
    /// `note_actor` for how a match is made, and `Ingest::link`'s doc
    /// comment for why the encounter graph is untouched by this: fight
    /// membership and store identity are separate concerns.
    pet_owner: HashMap<String, String>,
    /// Encounters where "You" has landed a confirmed hit on the fight's own
    /// anchor mob -- see `note_shared_target`. Once an id is in here, every
    /// future actor hitting that same anchor gets promoted inline; the
    /// retroactive sweep over everyone who hit it *before* "You" did only
    /// runs the moment an id is first inserted.
    you_confirmed_target_encs: HashSet<EncounterId>,
    /// Open "Quick Buff was just activated" windows, keyed by resolved
    /// activator name -> activation timestamp. See `note_quickbuff` and
    /// `flavor_evidence_for` for how these get consumed and pruned.
    pending_quickbuff: HashMap<String, Millis>,
    /// Quick Buff class evidence not yet committed, held for
    /// `GROUP_CAST_WINDOW_MS` in case it turns out to be a group cast
    /// coincidentally landing during the activator's own window -- see
    /// `attribute_flavor_hit`'s doc.
    pending_quickbuff_evidence: Vec<PendingQuickbuffEvidence>,
    /// Every recognized flavor landing (self and third-person alike),
    /// across every entity, from the trailing `GROUP_CAST_WINDOW_MS` --
    /// exists purely to answer "did this same text also land on someone
    /// else just now", the signal a group cast leaves. Deliberately
    /// separate from `effects` (which keeps full history, per-entity,
    /// never pruned): this only ever needs a few seconds of cross-entity
    /// lookback, so it's pruned on every touch instead.
    recent_flavor_landings: Vec<(Millis, u32, String)>,
    /// Log-time clock: set from the log's own timestamps while replaying
    /// history, then (once `mark_live` is called) also advanced by real
    /// elapsed time between ticks, so a fight that goes quiet during live
    /// tailing closes in near-real-time rather than only when the next
    /// line happens to arrive.
    log_clock: VirtualClock,
    last_wall_ms: Option<Millis>,
    /// `log_clock`'s value as of `last_wall_ms`. Snapshotting the two
    /// together, and only ever projecting forward from this pair rather
    /// than from a freshly-read `log_clock.now_ms()`, is what keeps a
    /// tick's wall-elapsed delta from being counted twice: a line arriving
    /// between two ticks already advances `log_clock` past this snapshot
    /// on its own (via `route`'s `set_at_least`), so the next tick's
    /// projection lands exactly on the already-advanced value instead of
    /// adding wall-elapsed on top of it. See `tick`.
    last_log_ms: Millis,
    live: bool,
    pub counts: LineCounts,
    /// Every unmatched-line shape seen this session, ranked by count --
    /// the Debug module's own "Unparsed" tab. Accumulated from both the
    /// live tail path (`route`) and backfill (`classify_chunk`'s
    /// per-thread copies, merged sequentially in `backfill_lines`), so a
    /// freshly-launched app replaying days of history sees the same
    /// picture the `eqlp coverage` CLI command would print for that file
    /// -- not just whatever arrives after this particular launch. See
    /// `crate::debugview`'s own doc for what the frontend does with this.
    unmatched_shapes: HashMap<Vec<u8>, ShapeStat>,
    unmatched_shapes_overflow: u64,
    shaper: Shaper,
    shape_scratch: Vec<u8>,
    pub recent: Vec<RecentLine>,
    /// Sound-notification-worthy events since the caller last drained
    /// this -- pure data, no I/O, same split `pending_history`/`pending_
    /// inventory_files` already use: `Ingest` only ever records that one
    /// happened, `tail_worker.rs` is what actually emits the Tauri event.
    /// Live-only, same gate `recent` uses -- see `crate::notifications`'s
    /// own doc for why a historical backfill replaying days of log
    /// shouldn't fire a burst of sounds for things that already happened.
    pub pending_notifications: Vec<crate::notifications::NotificationEvent>,
    /// One record per encounter closed since the caller last drained this.
    /// Pure data, no I/O -- built in `drain_closed`, persisted by whatever
    /// holds an `AppHandle` (`tail_worker.rs`), same split `recent` already
    /// uses for the live feed. See `crate::history`.
    pub pending_history: Vec<ParseRecord>,
    /// Filenames named by an "Outputfile Complete" line since the caller
    /// last drained this -- pure data, no I/O, same split `pending_history`
    /// already uses: `Ingest` only ever records *that* a dump exists and
    /// *what it's called*, never reads it (it doesn't know the base
    /// install folder the file actually lives in -- that's
    /// `AppConfig::base_dir`, a layer up). `tail_worker.rs` is what
    /// actually reads the file and acts on it.
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
            last_loc: None,
            last_teleport_cast: None,
            entered_via_teleport: None,
            last_origin_cast: None,
            learned_origin: None,
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
    /// Current position on the log's own clock -- milliseconds, no
    /// timezone, same basis as every `LocalTs` in `eqlp-core`.
    pub fn now_ms(&self) -> Millis {
        self.log_clock.now_ms()
    }

    /// When the *current* session began, for `overview::session`'s rate
    /// averaging -- the most recent `afk.off` line if there's been one,
    /// else `first_ts` (the start of this parse, "freshly logged on").
    /// `None` only before a single line has been processed.
    ///
    /// A return from AFK reads as a fresh session on purpose, not a
    /// continuation of whatever was running before: minutes (or hours) of
    /// AFK time sitting inside an unbroken "since log start" window would
    /// silently drag down every rate average computed over it -- plat/hour
    /// and xp/hour both go quietly wrong the moment idle time outweighs
    /// active time, which for a long real play session is common, not an
    /// edge case. Going AFK itself doesn't end anything here -- only
    /// coming *back* does, at the moment there's again real play to
    /// average -- so a still-AFK player's session start stays wherever it
    /// last was, not the moment they stepped away.
    pub fn session_start(&self) -> Option<Millis> {
        self.last_afk_off.or(self.first_ts)
    }

    /// Whether AFK as of the most recently processed line.
    pub fn currently_afk(&self) -> bool {
        self.afk_state
    }

    /// Zone difficulty tier (0-4) as of `ts`, parsed from whatever zone
    /// label was current at that instant. Stamped onto every pushed row
    /// (`Store::tier`) so a score baseline can later be scoped to "this
    /// target, this difficulty" with a plain `Filter`, not a query-time
    /// union over every past same-tier zone visit. See `crate::zone`'s doc
    /// for the naming convention this parses.
    fn current_tier(&self, ts: Millis) -> u8 {
        crate::zone::zone_tier(self.zone.at(ts).unwrap_or("")).1
    }

    /// `current_tier`'s sibling: the zone itself, interned, not just its
    /// difficulty digit. Stamped once onto an `Encounter` at open time
    /// (`Store::open_encounter`) rather than re-derived from `start_ms` on
    /// every later query that wants "what zone was this fight in" --
    /// `combat::list_zone_encounters` reads the stamped field, not this.
    /// Takes `&mut self` (unlike `current_tier`) because interning a new
    /// string needs it; the short-lived borrow from `self.zone.at(ts)` is
    /// released before that happens, via the owned `to_string()`, so the
    /// two don't conflict.
    ///
    /// Also primes `wiki_zone_cache` for this raw label right away
    /// (`resolved_wiki_zone`) -- eager, not lazy-on-first-query, so the
    /// resolution work for a zone visit happens once, here, while parsing,
    /// rather than needing every query against it to check "have we
    /// resolved this one yet" and possibly do it themselves.
    fn current_zone(&mut self, ts: Millis) -> Option<Sym> {
        let z = self.zone.at(ts)?.to_string();
        let sym = self.sym(&z);
        self.resolved_wiki_zone(sym);
        Some(sym)
    }

    /// The wiki zone `raw_zone` (an interned raw `zone.enter` label)
    /// resolves to, if any -- returns its stable `zonedata::Zone::id`
    /// (the wiki's own URL-slug identifier, e.g. `"Plane_of_Hate"`), not
    /// the display `name`. Deliberately an id, not a name: a name still
    /// needs a case-insensitive compare at every call site (spelling is
    /// spelling, casing drifts), while two ids either are the same zone or
    /// they aren't -- an exact `==` -- which also makes this directly
    /// checkable by eye: `debugview::list_debug_encounters` shows exactly
    /// this value next to each encounter, and a zone page's own id is
    /// right there in `zonedata::Zone::id` too, so "is this encounter
    /// really tagged with the zone I'm looking at" is a plain string
    /// comparison a person can do, not something to just trust.
    ///
    /// Checked against `wiki_zone_cache` first, computed via `zone::
    /// zone_matches` over `zonedata::zones()` only on a cache miss --
    /// runs at most once per *distinct* raw zone label a session ever
    /// sees (typically a couple hundred at most), no matter how many
    /// thousands of fights or queries share that label afterward.
    /// `cached_wiki_zone` is the read-only sibling query code should
    /// reach for instead -- this one can populate the cache, `&Ingest`-
    /// only query functions can't.
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

    /// Read-only lookup into `wiki_zone_cache` -- a `zonedata::Zone::id`,
    /// not a name, see `resolved_wiki_zone`'s doc for why. `None` here is
    /// ambiguous between "this raw zone matched nothing in the wiki
    /// guide" and "this raw zone was never resolved at all", but in
    /// practice the second case can't happen for any `Sym` reachable from
    /// an `Encounter`: `current_zone` calls `resolved_wiki_zone` on every
    /// one it ever stamps, before the encounter exists to be queried.
    pub fn cached_wiki_zone(&self, raw_zone: Sym) -> Option<&'static str> {
        self.wiki_zone_cache.get(&raw_zone).copied().flatten()
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

    /// One unmatched line, from the live tail path -- shapes it and folds
    /// it into `unmatched_shapes` directly (no per-thread copy to merge,
    /// unlike backfill's `classify_chunk`/`merge_unmatched_shape`).
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

    /// One already-shaped chunk result, from backfill's parallel classify
    /// step -- see `classify_chunk`'s own local accumulator and
    /// `backfill_lines`' sequential merge loop. `stat.count` folds in
    /// whole (that chunk may have seen this shape many times), and the
    /// *existing* example is kept on a hit -- first-seen, not
    /// last-merged, matching `note_unmatched_shape`'s own single-line
    /// behavior of never overwriting an example once set.
    fn merge_unmatched_shape(&mut self, shape: Vec<u8>, stat: ShapeStat) {
        if let Some(existing) = self.unmatched_shapes.get_mut(&shape) {
            existing.count += stat.count;
        } else if self.unmatched_shapes.len() < UNMATCHED_SHAPE_CAP {
            self.unmatched_shapes.insert(shape, stat);
        } else {
            self.unmatched_shapes_overflow += stat.count;
        }
    }

    /// Every unmatched-line shape seen this session, highest count first
    /// -- the Debug module's "Unparsed" tab, and the same ranking the
    /// `eqlp coverage` CLI command prints for the identical clustering.
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

    /// Unmatched *lines* dropped once `UNMATCHED_SHAPE_CAP` distinct
    /// shapes were already being tracked -- not itself a distinct-shape
    /// count, a line count, so it can be large even when the cap is only
    /// a little too small.
    pub fn unmatched_shapes_overflow(&self) -> u64 {
        self.unmatched_shapes_overflow
    }

    /// Call once per line, in order, with the already-computed classification.
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
                // Checked against the buff-landing flavor dictionary
                // first (unconditional -- not gated on an open Quick Buff
                // window; see `flavor_evidence_for`'s doc) -- a hit means
                // this line is *understood*, just not by a rule pattern,
                // so it has no business in the Debug module's "Unparsed"
                // shape list. Only a real miss on both checks still gets
                // shape-clustered for that list.
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

    /// Call once per worker loop tick, live or not. Advances the log clock
    /// during live idle stretches and closes fights that have gone quiet.
    ///
    /// Projects forward from `last_log_ms` (the log clock's value as of
    /// `last_wall_ms`), not from `log_clock.now_ms()` read fresh here --
    /// lines routed since the last tick may already have advanced the log
    /// clock past that snapshot via their own timestamps, and adding
    /// wall-elapsed on top of an already-advanced value would double-count
    /// the same span of real time. During any continuously-active session
    /// that double-count compounds every tick a line also arrived in,
    /// racing the log clock far ahead of real time and idle-closing fights
    /// that never actually went quiet.
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

    /// Executes one already-extracted action against the store/graph/zone/
    /// timeline. Never touches `line`/`Match`/`Engine` -- everything it
    /// needed was pulled out by `extract_action`, which is what lets the
    /// same logic run from a sequential merge after parallel classification
    /// (`backfill_lines`) as well as inline on the live tail thread.
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
                // A resisted spell deals no damage, so a damage line is
                // unambiguous proof of landing -- the one outcome this
                // module can confirm without a dedicated result line. Tags
                // outside SPELL (melee, procs, damage shields) never have a
                // cast pending under this name, so `confirm_landed` is a
                // harmless no-op for them.
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
            Action::Miss { src, dst, verb, flags } => {
                self.record_avoided(ts, &src, &dst, &verb, flag::MISSED | flags)
            }
            Action::Block { src, dst, verb, flags } => {
                self.record_avoided(ts, &src, &dst, &verb, flag::BLOCKED | flags)
            }
            Action::Dodge { src, dst, verb, flags } => {
                self.record_avoided(ts, &src, &dst, &verb, flag::DODGED | flags)
            }
            Action::Parry { src, dst, verb, flags } => {
                self.record_avoided(ts, &src, &dst, &verb, flag::PARRIED | flags)
            }
            Action::Death { victim } => self.record_death(ts, &victim),
            Action::Zone { zone } => {
                // why: stop fights bleeding across zone changes
                self.encounters.close_all(ts);
                self.entered_via_teleport = self
                    .last_teleport_cast
                    .clone()
                    .filter(|(cast_ts, _)| ts - cast_ts <= TELEPORT_WINDOW_MS)
                    .map(|(_, landing)| (ts, landing));
                // Origin's own real confirmation -- see `learned_origin`'s
                // own doc. Same window, same "last one wins" shape as the
                // wiki-fixed teleports above, just recording *which zone*
                // instead of looking one up, since there's nothing to look
                // up.
                if self.last_origin_cast.is_some_and(|cast_ts| ts - cast_ts <= TELEPORT_WINDOW_MS) {
                    self.learned_origin = Some((ts, zone.clone()));
                }
                self.zone.enter(ts, zone);
            }
            Action::LevelUp { level } => {
                self.levels.observe(ts, level);
            }
            Action::AaGained { name, rank, cost } => {
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
                // A pet's first action after being summoned is casting its
                // own spawn buff, "Inner Fire" specifically -- measured
                // against the real reference log (see PET_MATCH_WINDOW_MS),
                // not any cast in general. That distinction matters: a
                // version that treated *any* first-ever cast as a pet
                // candidate mismatched a real, not-yet-proven-player
                // character whose first cast happened to follow someone
                // else's pet summon by a second, at exactly the moment
                // that's most common -- session/zone-in, when everyone's
                // first buff and everyone's pet summon land in the same
                // few seconds. Checking only "Inner Fire" is what was
                // actually validated to be safe.
                if spell == "Inner Fire" {
                    self.note_actor(ts, &who);
                }
                // Recognized teleport casts are resolved against the
                // wiki-confirmed landing pack, not a name-shape guess --
                // see `teleportdata`'s own doc for why. "You" or a proven
                // ally, since the group-shaped siblings land the whole
                // group, not just the caster -- an unproven stranger's
                // cast deliberately does not count (see `is_ally`'s own
                // doc).
                if who == "You" || self.is_ally(&who, ts) {
                    if let Some(landing) = teleportdata::landing_for(&spell) {
                        self.last_teleport_cast = Some((ts, landing));
                    }
                }
                // Origin: personal only (see `last_origin_cast`'s own doc
                // for why this doesn't join the ally-aware check above).
                if who == "You" && spell == "Origin" {
                    self.last_origin_cast = Some(ts);
                }
                // Live spell rank: personal only, same reasoning as
                // Origin above -- this is for the player's own spellbook
                // display, not a general per-entity fact worth tracking
                // for every ally/mob that happens to cast something.
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
                let spell_sym = self.store.sym(base).0;
                self.casts.begin(ts, caster.0, spell_sym);
                self.classes.observe_cast(
                    caster.0,
                    self.zone.index_at(ts),
                    crate::classdata::classes_for(base),
                );
            }
            Action::CastResisted { spell } => {
                // The pattern hardcodes "resisted your", so the caster is
                // always the player -- the resister's name plays no role
                // in resolving the player's own pending cast, so
                // `extract_action` never even pulls it out of the match.
                let you = self.sym("You").0;
                let spell_sym = self.store.sym(base_spell_name(&spell)).0;
                self.casts
                    .resolve(ts, you, spell_sym, CastOutcome::Resisted);
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
            Action::CastBlocked { spell, target, blocker } => {
                // Deliberately doesn't call `self.casts.resolve(...)` --
                // "blocked by a stacking conflict" isn't the same failure
                // `CastOutcome::Resisted` means (a target's resist roll),
                // and folding it in would quietly skew whatever reads that
                // outcome as a resist-rate stat. No outcome variant fits
                // today, so this stays out of cast resolution entirely
                // rather than picking the least-wrong existing one.
                let you = self.sym("You").0;
                self.classes.observe_cast(
                    you,
                    self.zone.index_at(ts),
                    crate::classdata::classes_for(base_spell_name(&spell)),
                );
                if let Some(blocker) = blocker {
                    self.record_effect_ping(ts, &target, &blocker);
                }
            }
            Action::StateEffect { target, text } => self.record_effect_ping(ts, &target, &text),
            Action::PlayerLoc { x, y, z } => {
                self.last_loc = Some((ts, x, y, z));
            }
            Action::AbilityActivated { who, ability } => {
                let sym = self.sym(&who);
                self.classes.observe_cast(
                    sym.0,
                    self.zone.index_at(ts),
                    crate::classdata::classes_for(&ability),
                );
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
            Action::QuickBuff { who } => self.note_quickbuff(ts, &who),
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

    /// Pushes every cast the resolver has finished judging into the store as
    /// an `EventKind::Cast` row, outcome encoded in `flags` (`flag::CAST_*`).
    /// Called after every action (a resist/interrupt/fizzle/landed line can
    /// close a cast the same tick it arrives) and once per `tick` to catch
    /// expiry-driven `Unconfirmed` closures, which arrive with no line at
    /// all.
    ///
    /// `target` is set equal to `actor`: `cast.begin` never names a target
    /// in this log (see `eqlp_session::cast`'s doc comment), so there is no
    /// real value to put there. Revisit if a target ever becomes available.
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
            self.store.push(
                r.end_ms,
                EventKind::Cast,
                actor,
                actor,
                ability,
                0,
                flags,
                NO_ENCOUNTER,
                tier,
            );
        }
    }

    /// Damage is what defines the encounter graph (`docs/design/encounters.md`:
    /// "each damage line is an edge"), so this is the only event kind that
    /// opens a new fight. Everything else attaches to whatever fight is
    /// already open, if any.
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
        let enc = self.link(ts, src, dst);
        let a = self.sym(src);
        let t = self.sym(dst);
        self.note_shared_target(ts, enc, src, t);
        self.clear_dead_if_acting(ts, a);
        let ab = self.store.ability_id(ability, tags);
        let tier = self.current_tier(ts);
        let idx = self
            .store
            .push(ts, EventKind::Damage, a, t, ab, amount, flags, enc.0, tier);
        self.store.extend_encounter(enc, idx);
        self.drain_closed();
    }

    /// "If someone is able to damage the same target as me, they are in my
    /// party" -- the log gives no roster line, but landing damage on the
    /// very same mob in the very same fight "You" are also fighting is
    /// stronger, far more common evidence than chat (which a busy raid
    /// tank may never once use -- see the Monsters-module leak this was
    /// written to close). Promotes via `Entities::note_shared_target`, the
    /// same monotonic, sticky-forever mechanism `note_player_channel`
    /// already uses: once earned, `Kind::Player` is a permanent identity
    /// fact, same as proof-by-chat.
    ///
    /// Two paths: the moment "You" lands the hit that confirms this fight
    /// (`src` resolves to "You" and `dst` is this fight's own anchor),
    /// sweep back over everyone who already hit that anchor earlier in the
    /// same fight -- they were just as much a party member before "You" had
    /// proof as after, and were likely fighting before "You" even got
    /// there. After that, every future hit on the anchor promotes its actor
    /// inline, no sweep needed.
    ///
    /// Guards against two false positives: the anchor mob "hitting itself"
    /// (a reflected damage shield) never promotes, and a *currently
    /// charmed* actor never does either -- a charmed mob temporarily
    /// fighting on your side is already correctly handled by
    /// `Allegiance::of`'s `State::Charmed` flip, and promoting it to
    /// permanent `Kind::Player` here would outlive the charm, reading as an
    /// ally forever after it breaks.
    fn note_shared_target(&mut self, ts: Millis, enc: EncounterId, src: &str, dst_sym: Sym) {
        let Some(anchor) = self.store.encounter(enc).map(|e| e.target) else {
            return;
        };
        if dst_sym != anchor {
            return; // damage to something other than this fight's own mob proves nothing
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

    /// Shared guard for both `note_shared_target` promotion paths -- never
    /// promotes a *currently charmed* entity. See `note_shared_target`'s
    /// doc for why (a temporary ally must not become a permanent one).
    fn promote_party_member(&mut self, sym: Sym, ts: Millis) {
        if matches!(self.timeline.state_at(sym.0, ts), Some((State::Charmed, _))) {
            return;
        }
        let name = self.store.name(sym).to_string();
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

    /// A swing that dealt zero damage because it was fully avoided --
    /// miss, block, dodge, or parry (`mitigation` is the one matching
    /// `flag::MITIGATED` bit). Lands on the *same* ability row a landed
    /// swing of this `verb` would (`canonical_melee_ability`), tagged with
    /// which kind of avoidance it was, rather than a separate synthetic
    /// "Miss"/"Block"/"Dodge"/"Parry" ability -- see `flag::MITIGATED`'s
    /// own doc for why this belongs on the attacker's own swing, not
    /// invented as an ability the defender "used".
    fn record_avoided(&mut self, ts: Millis, src: &str, dst: &str, verb: &str, mitigation: Flags) {
        let enc = self
            .current_encounter_of(src)
            .or_else(|| self.current_encounter_of(dst));
        let a = self.sym(src);
        let t = self.sym(dst);
        self.clear_dead_if_acting(ts, a);
        let ab = self.store.ability_id(canonical_melee_ability(verb), tag::MELEE);
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
        // Resolve any XP gain still waiting on its own kill -- see
        // `pending_xp`'s doc. Must run after `self.encounters.death` (so
        // the encounter it just closed is findable) and before
        // `drain_closed` (so eviction can't remove it out from under this
        // lookup first).
        if let Some(p) = self.pending_xp.take() {
            if p.ts == ts {
                // Same second as this death -- consumed either way, found
                // or not: a gain that matches on timing but somehow can't
                // resolve to a real encounter isn't going to do better
                // waiting for some later, unrelated death instead.
                if let Some(id) = self.encounter_id_for_victim(victim) {
                    self.store.enc[p.row as usize] = id.0;
                }
            } else {
                // Not this death's to claim -- put it back for whatever
                // death (if any) actually shares its timestamp.
                self.pending_xp = Some(p);
            }
        }
        let sym = self.sym(victim);
        self.timeline.observed(ts, sym.0, State::Dead);
        self.drain_closed();
    }

    /// The most recently opened encounter targeting `victim` -- `record_
    /// death`'s own lookup for attributing a `pending_xp` row, right after
    /// `self.encounters.death` has already resolved which fight this death
    /// belongs to. Deliberately not `recent_encounter_for`: that one
    /// exists to match *loot* to kill order among several unclaimed
    /// same-named corpses, with claim-tracking side effects that make
    /// sense for loot specifically (see its own doc) but have no business
    /// here -- XP attribution runs at the exact moment a death resolves,
    /// when there's only ever one real answer (whichever encounter
    /// `self.encounters.death` just closed), so a plain reverse scan for
    /// the newest matching name is enough, and reusing loot's claim state
    /// would only risk the two features quietly interfering with each
    /// other over encounters neither actually shares.
    fn encounter_id_for_victim(&self, victim: &str) -> Option<EncounterId> {
        self.store
            .encounters
            .iter()
            .rev()
            .find(|e| self.store.name(e.target).eq_ignore_ascii_case(victim))
            .map(|e| e.id)
    }

    /// "You gain experience!" -- always self-directed (see `Action::Xp`'s
    /// doc), so `actor`/`target` are both "You"; `ability` reuses the
    /// interner to carry `scope` ("solo"/"party"/"group"/"raid") the same
    /// way `record_loot` reuses it for an item name -- not really an
    /// ability, but the column is already exactly "interned name -> per-
    /// row metadata", and a dedicated scope column for one row kind isn't
    /// worth adding. `amount` is `pct` in milli-percent (`11.000%` ->
    /// `11000`), not the bare percentage -- `Store::amount` is a `u64`, so
    /// this is what preserves the log's own three decimal digits without a
    /// new float column; divide by 1000.0 to get the percentage back.
    ///
    /// `enc` starts as `NO_ENCOUNTER` and is filled in later, if at all, by
    /// `record_death` -- unlike loot, there's nothing to search for yet at
    /// this point: a kill's XP line comes *before* its own "You have
    /// slain" line, not after, so the encounter that earned it may not
    /// even be closed (or exist as a lookup target) when this runs. See
    /// `pending_xp`'s own doc for the row-index handoff that makes the
    /// later backfill possible.
    ///
    /// Confirmed against the real log, not assumed: this line fires for
    /// both kill XP (near-always immediately above a matching "You have
    /// slain" line, same second) *and* quest turn-in XP ("You gain
    /// experience!" immediately followed by "You complete the trade with
    /// ..."). Only the first kind has a death line to attach to; the
    /// second is expected to stay `NO_ENCOUNTER` forever, and that's
    /// correct, not a gap -- there is no kill to blame it on.
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

    /// Platinum/gold/silver/copper actually received -- always the player,
    /// same self-directed shape `record_xp` has, and for the same reason
    /// (`ability` carries `source`, not a dedicated column -- see
    /// `EventKind::Currency`'s doc). `text` is parsed by `parse_currency_
    /// copper`; a line whose amount clause parses to nothing (`0`) is
    /// dropped rather than pushed as an empty row -- that only happens if
    /// the log phrases an amount this parser genuinely doesn't recognise,
    /// and a silent zero-value row would be worse than a silently missed
    /// one, since it would look like real data to every caller that reads
    /// `Store::amount` without checking.
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

    /// Linked to a *best-effort* `EncounterId` (`recent_encounter_for`,
    /// below), not left as `NO_ENCOUNTER`: by the time a corpse is
    /// looted, the kill that produced it has almost always already closed
    /// (`drain_closed`, via `record_death`), so there's no *live* fight to
    /// attach to the ordinary way `link` does for damage/heal/miss. Two
    /// mobs sharing a name fighting close together doesn't make "which
    /// one's corpse" a coin flip, though: `recent_encounter_for` matches
    /// loot to *kill order* (oldest still-unclaimed death first), not
    /// just recency -- see its own doc. `crate::monsters`' own mob-name
    /// aggregation doesn't depend on this at all (it groups by the row's
    /// own `target`, set from the corpse text a few lines down,
    /// independent of `enc`) -- this is for call sites that want "what
    /// did *this* pull drop", like `combat::encounter_detail`.
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
        // why: `sold` (the same "and sold it for..." clause `Action::
        // Loot::sold_for`'s presence already signals) means this exact
        // item never actually stuck around to turn in anywhere -- flagged
        // on the row itself rather than left to a separate same-timestamp
        // Currency-row correlation, which a busy multi-item corpse could
        // make ambiguous. See `flag::LOOT_AUTO_SOLD`'s own doc.
        let flags = if sold { flag::LOOT_AUTO_SOLD } else { 0 };
        self.store
            .push(ts, EventKind::Loot, looter, target, ab, qty, flags, enc, tier);
    }

    /// How long a gap between two loot lines against the same mob name
    /// still reads as "still working through the one corpse's loot
    /// window" rather than "moved on to a different one". Not just "how
    /// fast someone clicks" -- an advanced-loot item without an "Always"
    /// rule opens an interactive window that sits there until manually
    /// resolved (see `LOOT_GRACE_MS`'s doc for the same mechanic), so two
    /// items off the *same* corpse can legitimately land minutes apart if
    /// the player got pulled into something else between them. Set well
    /// Best-effort: which encounter a loot line against `mob` (at `ts`)
    /// belongs to. Matches *kill order*, not just recency: killing two
    /// same-named mobs close together isn't ambiguous if there's both a
    /// death count and a claim count to go on, and the naive version of
    /// this (always the single most-recent same-named encounter) breaks
    /// the instant a third same-named mob dies before the first corpse
    /// gets looted -- it would keep pointing at the newest death even
    /// while a player's still working through the oldest one's items.
    ///
    /// Two-part rule:
    /// 1. `loot_cursor`: if the encounter currently claimed for this
    ///    exact mob name is still within `combat::LOOT_GRACE_MS` of `ts`
    ///    -- judged off *that encounter's own* last activity, not off
    ///    the gap since the last loot line against this name -- reuse
    ///    it. This is what lets a slow manual loot-window resolution
    ///    (`combat::LOOT_GRACE_MS`'s doc: an advanced-loot item without
    ///    an "Always" rule can sit unresolved for many minutes) still
    ///    land on the right corpse. An earlier version instead tracked
    ///    "how long since the last loot line for this name" as its own
    ///    separate sticky window -- a second, independent guess layered
    ///    on top of this one, and it broke exactly that case: once that
    ///    unrelated gap lapsed, the re-search below would find this same
    ///    corpse already sitting in `loot_claimed` and wrongly skip past
    ///    it, even though the player was still mid-way through looting
    ///    it. Checking the encounter's own recency directly instead of a
    ///    proxy for it removes that whole failure mode.
    /// 2. Otherwise, advance: the OLDEST same-named encounter within
    ///    `combat::LOOT_GRACE_MS` of `ts` that hasn't already been
    ///    claimed (`loot_claimed`) by an earlier loot line. Marking it
    ///    claimed here is what lets a *later* same-named corpse resolve
    ///    to a genuinely different encounter instead of this one being
    ///    picked again.
    ///
    /// Known remaining trade-off: two same-named mobs killed close
    /// together, where the *first* one's loot window is left open for a
    /// long time while the second gets looted promptly, can still have
    /// the second one's loot line "win" the cursor and then, later, get
    /// re-claimed onto the first once the second ages out -- rule 1 only
    /// protects a single corpse being slowly resolved, not perfectly
    /// disambiguating several interleaved ones. That's the right thing
    /// to trade for: one corpse resolved slowly is the case this was
    /// actually reported against, and looting corpses out of kill order
    /// at all is the one thing a log alone can never fully resolve
    /// anyway.
    ///
    /// A full scan of `Store::encounters` per call, not an early-exit
    /// windowed search -- deliberately simple over clever: this runs
    /// once, at ingest time, not on a poll, and even a long session's
    /// encounter count (thousands, not millions) keeps this well under
    /// the cost that actually mattered elsewhere in this app (the full
    /// *event*-store scans fixed earlier). Worth revisiting only if a
    /// real session's ingest time measures as a problem, not
    /// pre-emptively.
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

    /// Resolves `name` to whatever casing this identity was first observed
    /// under (`Entities::display_name`), registering it with the entity
    /// table if this is the first time it's been seen through this path (a
    /// heal or miss can name someone before any damage line does). The
    /// canonical form everything else here keys on.
    fn resolve_name(&mut self, name: &str) -> String {
        self.encounters.entities.observe(name);
        self.encounters.entities.display_name(name).to_string()
    }

    /// Interns `name`, redirecting through inferred pet ownership first if
    /// this identity has been matched to an owner (`pet_owner`, populated
    /// by `note_actor`) -- so a merged pet's damage, heals, and misses all
    /// land on the owner's `Sym` everywhere, not just wherever it was first
    /// detected. Without the case-folding resolution this also does, the
    /// store's symbol table could split one entity into two syms over a
    /// sentence-position casing difference the same way the encounter
    /// graph used to -- see `docs/design/session.md`, "Case folding".
    fn sym(&mut self, name: &str) -> Sym {
        let resolved = self.resolve_name(name);
        let effective = self.pet_owner.get(&resolved).cloned().unwrap_or(resolved);
        self.store.sym(&effective)
    }

    /// Called only from the `Cast` action, deliberately not also from
    /// damage/heal/miss: a pet's first logged action is reliably its own
    /// spawn self-buff cast, and restricting candidacy to "first-ever
    /// cast" is what was actually measured against the real reference log
    /// (see `PET_MATCH_WINDOW_MS`) before this shipped. Checking every
    /// action type would catch a couple more real pets but was never
    /// validated against real data, and widens the population of
    /// first-time actors this runs against by orders of magnitude (every
    /// new player and mob in the whole log, not just new casters) for a
    /// case that's cheap to be conservative about instead.
    ///
    /// The first time a name casts, checks whether it's the pet a recent
    /// "<Owner> summons a <flavour>." line was waiting on: if so, every
    /// future `sym()` call for this name -- including the one about to
    /// happen for this very cast -- resolves to the owner instead.
    ///
    /// Matches against the *closest-in-time* pending summon, not against
    /// "exactly one pending, else give up": requiring the window to hold a
    /// single candidate sounded safer, but measuring it against the real
    /// reference log showed it was too strict to be useful -- a raid
    /// buffing up summons several pets within a few seconds of each other
    /// (each a real, resolvable match against its *own* nearest summon),
    /// and the game itself sometimes logs one summon twice at an identical
    /// timestamp, which alone was enough to make "exactly one" fail on
    /// cases with an unambiguous correct answer. Closest-in-time matched
    /// every case the stricter rule missed, including the two names this
    /// was built to fix, without producing an implausible pairing in a
    /// spot check of the first 20 resolved. Once a pending summon is used
    /// it's removed, so it can't also match a second, later name.
    fn note_actor(&mut self, ts: Millis, name: &str) {
        let resolved = self.resolve_name(name);
        if self.pet_owner.contains_key(&resolved) {
            return; // already resolved, nothing to check
        }
        if !self.seen_actors.insert(resolved.clone()) {
            return; // not their first time acting
        }
        // Never a pet: the log's own player, and anyone already proven a
        // player by a player-only chat channel. Without this, the worst
        // case for the closest-in-time match is exactly session/zone-in --
        // everyone's first cast (buffing up) and everyone's pet summon
        // cluster into the same few seconds, and "closest in time" doesn't
        // care whether the candidate is a person, only that it's new and
        // nearby. A real player who hasn't spoken yet still isn't
        // protected by this -- that half of the gap is the same
        // unspoken-NPC-vs-unspoken-player ceiling `list_allies` already
        // documents -- but "You" and anyone already known to be a player
        // are unambiguous and cost nothing to exclude outright.
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
        // No pending summons at all: leave unresolved.
    }

    /// A "<Owner> summons a <flavour>." line: registers `owner` as a
    /// pending candidate for whichever brand-new entity acts next. See
    /// `note_actor`.
    fn note_pet_summon(&mut self, ts: Millis, owner: &str) {
        let resolved = self.resolve_name(owner);
        self.pending_summons.push((ts, resolved));
    }

    /// "<Name> activates Quick Buff.": opens a window during which an
    /// unmatched line gets checked against the buff-landing dictionary --
    /// see `flavor_evidence_for` and `crate::flavordata`'s module doc.
    fn note_quickbuff(&mut self, ts: Millis, who: &str) {
        let resolved = self.resolve_name(who);
        self.pending_quickbuff.insert(resolved, ts);
    }

    /// Checks one unmatched line's exact text against the buff-landing
    /// flavor dictionary -- the live-tail path's entry point, where
    /// classification happens one line at a time inline and the lookup can
    /// be skipped outright on a miss. `backfill_lines`' parallel path can't
    /// take this shortcut (no `Ingest` access on the classify threads) and
    /// instead does the dictionary lookup itself during classification,
    /// landing directly on `record_effect_ping`/`attribute_flavor_hit` in
    /// the sequential merge -- see `Classified::SelfFlavorHit` and
    /// `Classified::ThirdPersonFlavorHit`.
    ///
    /// Three independent checks, in order, cheapest and most specific
    /// first:
    ///
    /// 1. Direct hit: `text` *is* a known first-person landing message
    ///    verbatim, so it's about "You". This does two independent things
    ///    -- unconditionally record it as a state ping on "You"
    ///    (`record_effect_ping` -- the message says nothing about who cast
    ///    it, but it's still real evidence of what's true about the player
    ///    right now), and *conditionally* offer it as class evidence via
    ///    `attribute_flavor_hit`, which only fires inside a still-open
    ///    Quick Buff window. Different questions, different safety rules.
    /// 2. Third-person possessive hit: see `third_person_flavor`
    ///    (`"<Name>'s ..."`).
    /// 3. Third-person conjugated hit: see `verb_conjugated_flavor`
    ///    (`"<Name> feels ..."`, no possessive).
    ///
    /// Checks 2 and 3 are never class evidence -- a third-person line
    /// doesn't even prove an *ally* cast it, let alone the log owner (a
    /// mob's own DoT lands the same way) -- only ever a ping on whoever it
    /// landed on.
    ///
    /// Returns whether any check hit -- callers use this to decide
    /// whether the line still belongs in the Debug module's "Unparsed"
    /// shape list (`note_unmatched_shape`): a line this recognizes is
    /// *understood*, just not by a rule pattern, so it has no business
    /// being reported as something needing a new rule.
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
        false
    }

    /// Records a recognized buff/effect landing as a timestamped ping on
    /// `target_name` -- resolved through inferred pet ownership like any
    /// other actor name (see `resolve_name`), since a third-person hit's
    /// captured name is exactly that: an arbitrary entity name pulled out
    /// of log text, not a trusted symbol yet. Unconditional -- see
    /// `Effects`' own doc for why this doesn't need (or get) the Quick
    /// Buff gating `attribute_flavor_hit` requires.
    ///
    /// Also the *other* half of `attribute_flavor_hit`'s group-cast check
    /// (see that method's doc): every landing, on every entity, passes
    /// through here, so this is where a group cast reveals itself to
    /// whatever pending Quick Buff evidence it disproves -- checked in
    /// both time directions, since the group-mate's own landing can land
    /// in the log before or after the one that opened the pending entry.
    fn record_effect_ping(&mut self, ts: Millis, target_name: &str, text: &str) {
        let resolved = self.resolve_name(target_name);
        let sym = self.sym(&resolved).0;
        self.effects.push(sym, ts, text.to_string());

        // Cancel: this landing disproves any pending entry for the same
        // text nearby in time, two ways -- on a *different* entity (a
        // group cast), or on the *same* entity again (a maintained buff
        // pulsing, not a one-shot Quick Buff proc). `ts != p.ts` excludes
        // the landing that created the pending entry from cancelling
        // itself.
        self.pending_quickbuff_evidence.retain(|p| {
            if p.text != text {
                return true;
            }
            let group_cast = p.who != sym && (ts - p.ts).abs() <= GROUP_CAST_WINDOW_MS;
            let pulsing = p.who == sym && ts != p.ts && (ts - p.ts).abs() <= PULSE_WINDOW_MS;
            !(group_cast || pulsing)
        });
        // Commit: pending entries whose cancellation window has fully
        // closed (the longer of the two checks above, since either can
        // still cancel right up to its own deadline) with nothing showing
        // up to disprove them.
        let (still_pending, ready): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending_quickbuff_evidence)
            .into_iter()
            .partition(|p| ts - p.ts <= PULSE_WINDOW_MS);
        self.pending_quickbuff_evidence = still_pending;
        for p in ready {
            self.classes
                .observe_cast(p.who, self.zone.index_at(p.ts), p.classes);
        }

        self.recent_flavor_landings.retain(|(t, ..)| ts - *t <= GROUP_CAST_WINDOW_MS);
        self.recent_flavor_landings.push((ts, sym, text.to_string()));
    }

    /// The order-dependent half: *tentatively* attributes `classes` as
    /// evidence for whoever's Quick Buff window is still open at `ts` --
    /// but only when *exactly one* window is open. Two or more overlapping
    /// activations (a raid quickbuffing together) make "whose buff is
    /// this" genuinely ambiguous, and attributing it to the wrong
    /// activator is worse than not attributing it at all, the same
    /// reasoning `flavordata`'s module doc applies to trusting the message
    /// text in the first place.
    ///
    /// That safety check alone isn't enough, though -- it only guards
    /// against *multiple people* Quick Buffing at once, not against an
    /// unrelated caster's own group-wide buff coincidentally landing on
    /// the activator during their own window, which happens constantly in
    /// practice (everyone tends to buff up at the same moment before a
    /// pull). Confirmed as a real false positive against the reference
    /// log: a group buff landed on the player and three named allies in
    /// the same log-second, 3 seconds after the player's own Quick Buff
    /// activation, well inside `QUICKBUFF_WINDOW_MS` -- 110 of 240 real
    /// activations had *some* group-cast text land in their window. Quick
    /// Buff only ever affects its own activator, so the same text landing
    /// on someone else at nearly the same instant is proof positive it
    /// wasn't a personal Quick Buff proc.
    ///
    /// That still isn't the whole story -- a *single-target* ally buff
    /// maintained on just the player (a bard repeatedly re-singing one
    /// song on the group's main damage dealer, say) never lands on anyone
    /// else at all, so the cross-entity check above can't see it either.
    /// It has its own real signature, though: Quick Buff applies once at
    /// activation and doesn't recur, while a maintained song pulses on a
    /// short, regular cadence. Confirmed directly against the reference
    /// log: "You feel an aura of mystic protection surrounding you."
    /// pulsing on the player at 17:07:23, :29, :35, :41, :47, :53 -- a
    /// steady ~6s cadence sustained for minutes, landing on no one else
    /// at any point in 2.8M lines, yet 4,180 real occurrences against
    /// only 240 real Quick Buff activations total. See `PULSE_WINDOW_MS`.
    ///
    /// So this doesn't commit immediately -- it queues a
    /// `PendingQuickbuffEvidence` (first checking whether the text has
    /// *already* landed on someone else moments ago via
    /// `recent_flavor_landings`, or on the *same* entity moments ago via
    /// `effects` -- either one disqualifies it before it's even queued)
    /// and lets `record_effect_ping` cancel it later if either check's
    /// evidence shows up after the fact instead, or commit it if neither
    /// does.
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
            .filter(|&&t| t == text)
            .count()
            > 1; // > 1: `text`'s own current landing is already in there
        if group_cast_already || already_pulsing {
            return;
        }
        self.pending_quickbuff_evidence.push(PendingQuickbuffEvidence {
            ts,
            who: sym,
            classes,
            text: text.to_string(),
        });
    }

    /// `name` resolved through inferred pet ownership, for callers outside
    /// `Ingest` that walk `entities_by_enc` -- that list is the encounter
    /// graph's raw entity names, untouched by pet merging (see `link`'s
    /// doc comment on why), so a caller displaying or querying by name
    /// needs this to land on the same identity `sym()` would have used.
    pub fn effective_name(&self, name: &str) -> String {
        let resolved = self.encounters.entities.display_name(name).to_string();
        self.pet_owner.get(&resolved).cloned().unwrap_or(resolved)
    }

    /// How many pets have been matched to an owner so far -- surfaced in
    /// the Overview module so the inference is visible, not silent.
    pub fn pet_owner_count(&self) -> usize {
        self.pet_owner.len()
    }

    /// Routes one damage edge through the encounter graph, then resolves it
    /// to a store `EncounterId`, opening one the first time this graph
    /// component is seen.
    ///
    /// A merged-away graph component (`graph.rs`'s `merge`) doesn't share
    /// `store::Encounter`'s single contiguous row range with the survivor,
    /// so it keeps its own store-side counterpart -- `merge` pushes a
    /// `Closed` record for it directly (not via `close`, which only runs
    /// for ids still in `live`), so `drain_closed` still ends it instead of
    /// leaving it open forever. See `graph.rs::merge`'s own comment.
    fn link(&mut self, ts: Millis, actor: &str, target: &str) -> EncounterId {
        let enc_id = self.encounters.damage(ts, actor, target);

        // Whichever side of *this* edge is provably not the mob -- "You"
        // checked literally alongside proven identity, since `Kind::Player`
        // for "You" itself is only proven once the log owner has spoken on
        // a player channel, and a fresh session where they haven't yet must
        // not lose that guarantee. Identity alone isn't enough, though: a
        // proven player or pet who is *currently charmed* is fighting for
        // the other side for as long as that lasts, so this checks
        // `Allegiance::of(kind, state)` -- kind plus state *as of this
        // edge's own timestamp* -- not raw `Kind`, the same reasoning
        // `list_allies`/`fight_state_at` already apply per-query rather
        // than trusting identity alone. An encounter this fight-scoped
        // check ever anchors on a charmed ally is correct for exactly this
        // fight; if the charm breaks and they go back to being themselves,
        // that's a different moment with its own state, not a reason to
        // revisit this one. Still not exhaustive: an ally who has neither
        // spoken on a player channel nor summoned a pet stays
        // `Kind::Unproven` and reads exactly like a real mob would -- the
        // same unspoken-NPC-vs-unspoken-player ceiling `list_allies` and
        // `note_actor` already document, not a new gap.
        let actor_ally = self.is_ally(actor, ts);
        let target_ally = self.is_ally(target, ts);
        // `None` when both sides look like allies (self-inflicted damage --
        // "Wubble hit Wubble... by Cannibalize" -- or ally-on-ally noise)
        // or both look like mobs (rare: two unproven names fighting each
        // other, neither of them "You"): this edge has no opinion on which
        // side is the mob.
        let mob_side = match (actor_ally, target_ally) {
            (false, true) => Some(actor),
            (true, false) => Some(target),
            _ => None,
        };

        let store_id = if let Some(&id) = self.enc_map.get(&enc_id) {
            // Already open, anchored on whatever the *first* edge of this
            // fight had to guess from -- sometimes wrong: a boss's opening
            // swing lands on a groupmate who hasn't spoken yet, anchoring
            // the fight on them, before "You" or anyone else already proven
            // lands a hit on the actual boss moments later (a raid tank
            // silently eating hundreds of hits from a named boss for the
            // fight's whole duration is exactly this, and common). If the
            // current anchor reads as a proven ally and this edge names an
            // unambiguous non-ally, that beats whatever opened the fight --
            // retarget rather than leave a stale, wrong label for the rest
            // of it. Never retargets *away* from an already-good anchor:
            // `mob_side` is `None` for a merely-ambiguous later edge, so an
            // early correct guess is never second-guessed.
            if let Some(mob) = mob_side {
                let anchor_is_stale = self
                    .store
                    .encounter(id)
                    .map(|e| self.store.name(e.target).to_string())
                    .is_some_and(|name| self.is_ally(&name, ts));
                if anchor_is_stale {
                    let sym = self.sym(mob);
                    self.store.retarget_encounter(id, sym);
                }
            }
            id
        } else {
            // First edge of a brand new fight: nothing else is known yet,
            // so this edge's own guess is all there is. Falls back to
            // `target` when both sides look ambiguous -- see `mob_side`'s
            // doc -- and a later edge can still correct it above once
            // better evidence arrives.
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

    /// Ally as of `ts`, not ally forever -- `Allegiance::of(kind, state)`,
    /// not raw `Kind`, so a proven player or pet who is *currently charmed*
    /// reads as the enemy side for exactly as long as that lasts. "You" is
    /// certain `Kind::Player` even before the log owner has ever proven it
    /// by speaking (checked literally, same reasoning `link`'s doc comment
    /// gives); everyone else's `Kind` comes from accumulated evidence, same
    /// as ever. Shared by `link`'s new-fight and retarget paths so they can
    /// never disagree about what "ally" means.
    fn is_ally(&self, name: &str, ts: Millis) -> bool {
        let kind = if name.eq_ignore_ascii_case("you") {
            Kind::Player
        } else {
            self.encounters.entities.kind(name)
        };
        // Read-only: resolved through the same canonical casing `sym()`
        // interns under, without interning anything new here -- an ally
        // check must never itself create identity. A name with no `Sym`
        // yet (never seen before) has no recorded state either, which
        // defaults correctly to `Engaged`: never seen means never charmed.
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

    /// Syncs newly-closed graph encounters into the store. `Builder::closed`
    /// only grows, so this drains what's new since the last call.
    fn drain_closed(&mut self) {
        while self.closed_seen < self.encounters.closed.len() {
            // Cloned rather than borrowed: everything below needs &mut self
            // (sym() touches both the entity table and the store), which
            // can't coexist with a borrow into `encounters.closed`.
            let c = self.encounters.closed[self.closed_seen].clone();
            if let Some(&store_id) = self.enc_map.get(&c.id) {
                // why: `c.slain` mixes both sides -- an ally (or "You") dying
                // closes the fight the same as a real target death, so a
                // confirmed kill needs an actual enemy name in there, not
                // just *any* name.
                let confirmed_kill = c.slain.iter().any(|n| !self.is_ally(n, c.end_ms));
                let wiped = !confirmed_kill && c.slain.iter().any(|n| self.is_ally(n, c.end_ms));
                self.store
                    .close_encounter(store_id, c.end_ms, confirmed_kill, wiped);
                self.record_history(store_id, &c, confirmed_kill);
            }

            // Everything that leaves a closed fight alive and unaccounted
            // for left for a reason the log didn't report -- memory blur,
            // pacify, fleeing. Marked Lost/Inferred rather than left
            // looking Engaged forever. Players are excluded: the player
            // ending a fight is not "lost". See docs/design/timeline.md,
            // "Observed vs inferred".
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

    /// Builds one `ParseRecord` for a just-closed encounter and queues it in
    /// `pending_history`. Scoped to the player's own damage only -- see
    /// `ParseRecord::player_damage`'s doc for why a team total would be the
    /// wrong number here. No I/O: see `crate::history` for who actually
    /// persists these.
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
            // The player dealt no damage in this fight (a bystander to
            // someone else's pull, a buff-only presence) -- nothing to
            // record as "how did I do".
            return;
        }
        let zone = self.zone.at(c.start_ms).unwrap_or("Unknown").to_string();
        // Scoped to this same target at this same difficulty tier, not
        // every fight ever: a nuke's expected damage depends on both the
        // target's own resists and the zone's tier (see `crate::zone`), so
        // scoring a Tier 4 kill against an all-tiers average for that mob
        // made a perfectly normal parse look like it underperformed for a
        // reason that had nothing to do with how well it was played.
        // Self-diluted the same way as before -- this same encounter's own
        // hits are part of the baseline it's scored against, since there is
        // no cheap way to exclude just one encounter from the aggregate.
        // See `ParseRecord::score_ratio`'s doc.
        //
        // Skipped entirely during backfill (`!self.live`): the baseline
        // query has no `.encounter()` bound, so it scans the *whole* store
        // -- fine once per live close, but a backfill can close thousands
        // of encounters against a store that's still growing toward
        // millions of rows, and computing an O(store length) baseline that
        // many times over is exactly the quadratic-shaped cost that made a
        // big log's initial replay crawl. A backfilled record just carries
        // no score yet (`None`, the same value already shown whenever there
        // wasn't a baseline to score against) instead of computing one no
        // one can see until backfill finishes anyway; live closes -- one at
        // a time, arriving no faster than the game writes them -- still
        // score normally, since that per-close cost was never the problem.
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

        // The player's confirmed classes for *this fight's own zone visit*,
        // as of exactly this point in the sequential replay, right as this
        // encounter closes: every cast that happened during this visit has
        // already been applied to `self.classes`, so this is the honest
        // "as of this fight, in this visit" answer -- see
        // `classdetect::Detector::configuration_of_visit`'s doc. Already
        // alphabetical (the detector groups by a sorted set internally),
        // so two fights under the same configuration always produce the
        // same key regardless of which order the classes were confirmed
        // in, and group together in `crate::history::by_loadout`.
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

/// One line's meaning, fully extracted to owned data -- independent of the
/// `Match`/`line` it came from, so it can cross a thread boundary. Produced
/// by `extract_action`, consumed by `Ingest::apply`.
enum Action {
    Damage {
        src: String,
        dst: String,
        ability: String,
        tags: Tags,
        amount: u64,
        flags: Flags,
    },
    /// `dst` may still be a reflexive pronoun ("himself") -- resolved in
    /// `apply`, not here; extraction stays a pure read of what the line
    /// literally says.
    Heal {
        src: String,
        dst: String,
        ability: String,
        amount: u64,
    },
    /// `verb` is the swing's own attack-type ("punch", "slash", ...) --
    /// carried through so a fully-avoided swing lands on the *same*
    /// ability row a landed one of the same type would (`record_miss`),
    /// not a separate synthetic bucket. `flags` is the same free-text
    /// trailing flag `melee.hit` already carries, pre-parsed the same way
    /// ("Riposte", "Rampage", "Flurry", ...) -- an avoided swing can
    /// trigger a special attack type same as a landed one; see
    /// `record_avoided`.
    Miss {
        src: String,
        dst: String,
        verb: String,
        flags: Flags,
    },
    /// A swing the target actively blocked -- same shape as `Miss`, kept
    /// distinct so per-source accuracy can say "blocked" instead of
    /// folding it into a plain miss (see `record_block`).
    Block {
        src: String,
        dst: String,
        verb: String,
        flags: Flags,
    },
    /// Same as `Block`, for a dodge instead (see `record_dodge`).
    Dodge {
        src: String,
        dst: String,
        verb: String,
        flags: Flags,
    },
    /// Same as `Block`, for a parry instead (see `record_parry`).
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
    /// `level.up`: "You have gained a level! Welcome to level N!" -- always
    /// the player, first-person only, no third-person shape exists for
    /// this line. This is the player's *effective* (account) level, not
    /// any one class's own -- see `Ingest::levels`'s doc for how that
    /// distinction matters once a loadout swap can drop it back down.
    LevelUp {
        level: u8,
    },
    /// `aa.gained` (rank always 1, the line itself never states a number)
    /// or `aa.improved` (rank 2+, parsed from the line's own trailing
    /// digit). See `AaGrant`'s doc for why rank 1 is synthesized rather
    /// than read.
    AaGained {
        name: String,
        rank: u8,
        cost: u32,
    },
    /// `spell.scribe_start`/`spell.memorize_start` -- a "Beginning to..."
    /// line, proof of a Possible-tier spell. See `SpellLog`'s own doc.
    SpellBegan {
        name: String,
    },
    /// `spell.scribe_done`/`spell.memorize_done` -- a "finished..." line,
    /// proof of a Known-tier spell.
    SpellFinished {
        name: String,
    },
    Cast {
        who: String,
        spell: String,
    },
    /// `spell.resisted`. The pattern hardcodes "resisted your", so the
    /// caster is always the player. The resister's name is in the pattern
    /// (`who`) but plays no role in resolving the player's own pending
    /// cast, so it's never extracted here -- see the matching comment on
    /// `Ingest::apply`'s `CastResisted` arm.
    CastResisted {
        spell: String,
    },
    /// `cast.interrupted`. `source` is already resolved to a bare name --
    /// "You" if the line's `Your` branch matched, otherwise the caster's
    /// name with the possessive stripped by the pattern itself. See
    /// `extract_action`'s "cast.interrupted" arm.
    CastInterrupted {
        source: String,
        spell: String,
    },
    /// `cast.fizzled`, same source shape as `CastInterrupted`.
    CastFizzled {
        source: String,
        spell: String,
    },
    /// `cast.blocked`: "Your <spell> spell did not take hold on <target>.
    /// (Blocked by <blocker>.)" -- always the player's own cast (the game
    /// only ever tells *you* this about your own spells), so `spell` is
    /// real, high-confidence class evidence the same way a successful
    /// "begins casting" line is. `blocker`, when present, names a buff
    /// already active on `target` (a stacking conflict, not a resist) --
    /// real state, fed to `Effects` the same as any other recognized
    /// fact about an entity. `blocker` is `None` for the real lines that
    /// have no trailing parenthetical at all.
    CastBlocked {
        spell: String,
        target: String,
        blocker: Option<String>,
    },
    /// `state.you_poisoned`/`state.poisoned`/`state.you_diseased`/`state.
    /// diseased`: a named condition landing on `target`, fed to `Effects`
    /// the same as any other recognized fact -- these lines were already
    /// matched (kind "state") but produced no `Action` at all before this
    /// existed to feed. `text` is a fixed label ("Poisoned"/"Diseased"),
    /// not scraped flavor text -- there's no landing-message variety to
    /// preserve here, just which condition it is.
    StateEffect {
        target: String,
        text: String,
    },
    /// `state.location`: "Your Location is X, Y, Z." -- the `/loc`
    /// command's own output. Always the log owner (there's no
    /// third-person form of this line). Does NOT share `mapsdata.rs`'s
    /// parsed map files' own axis order -- see `Ingest::last_loc`'s own
    /// doc for the real mapping (verified against real dual data, not
    /// assumed) and MapViewer.svelte for where it's applied.
    PlayerLoc {
        x: f64,
        y: f64,
        z: f64,
    },
    /// `ability.activated`: "<who> activates <ability>." -- almost always
    /// third-person (see the pack rule's own note), so `who` is real
    /// class evidence for *that entity*, not necessarily "You". Also fed
    /// to `Effects` as a self-directed state fact on `who` (what's now on
    /// their weapon), independent of whether `ability` is one
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
    /// Charm wearing off, or the player's own mez ending -- both a return
    /// to `State::Engaged`.
    Recovered {
        who: String,
    },
    /// `loot.self`: "You have looted <item> from <corpse>." Always the
    /// player -- the log has no third-person loot line for anyone else.
    /// `corpse` is the raw capture, still carrying its `'s corpse` suffix;
    /// stripped in `record_loot`, not here, for the same reason `apply`
    /// (not `extract_action`) resolves `Heal`'s reflexive pronoun -- keep
    /// extraction a pure read of what the line literally says. `qty` is `1`
    /// for the singular "a"/"an" phrasing (the pattern's `qty` group never
    /// participates), or the stack size for "You have looted 2 X from...".
    /// `sold_for` is `loot.self.direct`'s own optional capture -- present
    /// only when the trailing clause was an auto-sell ("...and sold it for
    /// <denominations>."), raw and unparsed for the same reason `corpse`
    /// keeps its suffix here (see `record_currency`/`parse_currency_
    /// copper` for where the actual parsing happens).
    Loot {
        item: String,
        corpse: String,
        qty: u64,
        sold_for: Option<String>,
    },
    /// `xp.gain`: "You gain (party |group |raid |)experience! (X.XXX%)".
    /// Always the player -- the log never reports anyone else's XP.
    /// `scope` is the raw capture, empty string for solo (the pattern's
    /// first alternative) rather than the literal word "solo" -- normalized
    /// in `record_xp`, not here, same "extraction stays a pure read" reason
    /// `Loot`'s own doc gives for not stripping `corpse`'s suffix here
    /// either.
    Xp {
        scope: String,
        pct: f64,
    },
    /// Platinum/gold/silver/copper actually received, from either
    /// `money.corpse` ("You receive <amount> from the corpse.", `source`
    /// = `"corpse"`) or `money.vendor_sell` ("You receive <amount> from
    /// <vendor> for <item>(s).", `source` = `"vendor"`) -- `loot.self.
    /// direct`'s own auto-sell case goes through `Loot`'s `sold_for`
    /// instead (`apply` synthesizes `source = "autosell"` for that one),
    /// since it's one line producing two real facts (an item looted *and*
    /// currency earned), not two separate lines. `text` is the raw
    /// denomination list, unparsed -- see `Ingest::parse_currency_copper`.
    Currency {
        source: String,
        text: String,
    },
    /// `afk.on`/`afk.off` -- "You are now/no longer A.F.K. (Away From
    /// Keyboard)." No fields: the line carries nothing but the fact and
    /// its own timestamp, both of which `apply` already has.
    AfkOn,
    AfkOff,
    /// "Outputfile Complete: <file>" -- the client's own confirmation that
    /// a `/outputfile` command finished writing. See `Ingest::pending_
    /// inventory_files`'s doc for why this only records the filename and
    /// doesn't itself touch disk.
    OutputfileComplete {
        file: String,
    },
    /// `proc.item`'s `effect == "Exaltation"` case: "Your <item>
    /// (Exaltation) <flavor text>." -- proof that `item`'s Proc
    /// exaltation socket is genuinely live. See `ExaltationProcs`' own
    /// doc for why this is the only per-item exaltation fact this app
    /// can ever confirm.
    ExaltationProc {
        item: String,
    },
}

/// Classifies what one matched line means, without mutating anything. A
/// pure function of the rule pack and the match, which is what lets it run
/// on a worker thread during parallel backfill just as well as inline on
/// the live tail thread -- see `backfill_lines`.
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
            // No caster named -- the log gives us nothing to link this to.
            // Attributed to a placeholder rather than dropped, so the
            // damage still counts against the target's total.
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
            // The caster is named via the possessive "your" in the line
            // itself, not a separate field -- always "You" (this shape
            // never occurs for anyone else's own damage; see the pack
            // rule's own note).
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
            // `source` names the effect, not the entity: "Tranixx Darkpaw's
            // flames", "YOUR thorns" -- always the shield wearer's name (or
            // "YOUR" for the player) plus a possessive and the shield's
            // flavour word. Split it so the wearer is the actor, like every
            // other damage line, instead of "Tranixx Darkpaw's flames"
            // silently interning as its own entity separate from Tranixx
            // Darkpaw.
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
            let (src, dst, verb) = (str_field("source")?, str_field("target")?, str_field("verb")?);
            let flags = str_field("flag").map(|s| flag::parse(&s)).unwrap_or(0);
            Some(Action::Miss { src, dst, verb, flags })
        }
        "melee.blocked" => {
            let (src, dst, verb) = (str_field("source")?, str_field("target")?, str_field("verb")?);
            let flags = str_field("flag").map(|s| flag::parse(&s)).unwrap_or(0);
            Some(Action::Block { src, dst, verb, flags })
        }
        "melee.dodged" => {
            let (src, dst, verb) = (str_field("source")?, str_field("target")?, str_field("verb")?);
            let flags = str_field("flag").map(|s| flag::parse(&s)).unwrap_or(0);
            Some(Action::Dodge { src, dst, verb, flags })
        }
        "melee.parried" => {
            let (src, dst, verb) = (str_field("source")?, str_field("target")?, str_field("verb")?);
            let flags = str_field("flag").map(|s| flag::parse(&s)).unwrap_or(0);
            Some(Action::Parry { src, dst, verb, flags })
        }
        "cast.begin" | "sing.begin" => {
            let who = str_field("source")?;
            let spell = str_field("spell").or_else(|| str_field("song"))?;
            Some(Action::Cast { who, spell })
        }
        "spell.resisted" => Some(Action::CastResisted {
            spell: str_field("spell")?,
        }),
        "cast.blocked" => Some(Action::CastBlocked {
            spell: str_field("spell")?,
            target: str_field("target")?,
            blocker: str_field("blocker"),
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
            // `source` doesn't participate when the `Your` branch of the
            // pattern matched -- that's the only way it can be absent, so
            // defaulting to "You" is exact, not a guess.
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
            // Synthesised, not read from the log -- fold_key makes it match
            // whatever casing "you"/"You" was seen under.
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
        // Two entirely different client-generated lines for the same fact.
        // loot.self is the "--You have looted X from Y's corpse.--" form;
        // loot.self.direct is everything else the client says when you
        // loot something -- no "--" bracketing, "looted" not "have
        // looted", and a trailing clause that varies by what happened to
        // the item after (sold to a vendor automatically, banked to
        // currency, stored in a tradeskill depot or Dragon Hoard,
        // combined to create a higher tier) -- see that rule's own doc in
        // eql.toml for why one shared, permissive pattern beats a rule per
        // trailing clause. Both produce the identical Action::Loot; which
        // one fired doesn't matter past this point.
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
        // why: proc.item is generic (any "Your X (Y) Z." line) -- only
        // the Exaltation-labeled case is this app's own signal for a
        // live Proc exaltation socket. Every other `effect` value
        // (there's no confirmed real one yet -- see ExaltationProcs'
        // own doc) is left unrecorded rather than guessed at.
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
            // Only the channels that are provably player-to-player.
            // `says`/`shouts`/`auctions` are excluded on purpose -- NPCs
            // use `says` too, so it proves nothing. See
            // docs/design/encounters.md, "Entity classification".
            let (who, chan) = (str_field("who")?, str_field("chan")?);
            let player_only = matches!(
                chan.as_str(),
                "tells you"
                    | "tells the guild"
                    | "tells the group"
                    | "tell your party"
                    | "tell the guild"
                    | "tell the group"
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

/// Full spell names that end in what looks like a rank numeral but are not
/// rank-suffixed at all -- the numeral is part of the spell's own identity
/// (`Yaulp`, `Yaulp II`, `Yaulp III` are three different spells; EQL's own
/// rank system then nests a *second* numeral on some of them, e.g. `Yaulp II
/// I` for rank I of `Yaulp II`), or the wiki bakes a numeral into even the
/// lowest rank's title (`Rune I` has no bare `Rune` page at all). Naively
/// stripping either merges two real, different spells into one identity, or
/// invents a "base" name that doesn't exist -- confirmed against every
/// `cast.begin` spell name in the reference log cross-referenced with a
/// scrape of eqlwiki.com's spell pages (`~/eql/spells.json` on the
/// scraping machine; see `~/eql/scrape_eqlwiki_spells.py`). Regenerate by
/// re-running that cross-reference after a fresh scrape -- this list is a
/// snapshot, not derived at build time, so it goes stale if the wiki's
/// spell catalog changes.
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

/// Strips a trailing rank numeral ("Mesmerization IX" -> "Mesmerization"),
/// standing in for real rank recovery (`BACKLOG.md`, "Rank recovery") so a
/// `cast.begin` line's ranked spell name can be compared against a landed
/// damage/heal line's unranked one in `eqlp_session::cast::Resolver`. A
/// rank-1 cast has no numeral and passes through unchanged. Checked against
/// `PROTECTED_SPELL_NAMES` first -- see its doc comment for why a handful of
/// real spell names must never be stripped. Not a full solution otherwise --
/// see the caveat in `eqlp_session::cast`'s module doc.
fn base_spell_name(name: &str) -> &str {
    if PROTECTED_SPELL_NAMES.contains(&name) {
        return name;
    }
    match name.rsplit_once(' ') {
        Some((base, tail)) if is_roman_numeral(tail) => base,
        _ => name,
    }
}

fn is_roman_numeral(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| matches!(b, b'I' | b'V' | b'X' | b'L' | b'C' | b'D' | b'M'))
}

/// Splits a live cast-line spell name into its catalog base name and an
/// observed live rank, if any -- e.g. "Ice Comet X" -> ("Ice Comet",
/// Some(10)). This game has two unrelated roman-numeral phenomena on
/// spell names, confirmed against real data, not assumed: (1) a spell
/// *line* where the numeral is baked into the wiki's own canonical name
/// and names a wholly separate spell, not a rank of anything --
/// `packs/spells.json` has no bare "Monster Summoning" at all, only
/// "Monster Summoning II"/"III" as their own real entries, same as
/// "Yaulp"/"Yaulp II"/"Yaulp III" (`PROTECTED_SPELL_NAMES` above guards
/// `base_spell_name` against exactly this, but only for the handful of
/// names that specific function's own reference-log cross-check happened
/// to need -- not exhaustive, and not this function's job to reuse,
/// since a stale/incomplete guard list is exactly how a display bug
/// slipped through once already); (2) a live, per-character rank the
/// game appends only in combat-log text, never in the wiki's page title
/// at all (confirmed: "Ice Comet" is the sole `packs/spells.json` entry,
/// "Ice Comet X" never appears there, yet is real and common in the
/// reference log -- see `SpellRanks`' own doc). Disambiguated here by
/// checking the catalog directly rather than any hand-curated list: try
/// the *full* name first -- if it's already real, the numeral is
/// identity, no rank. Only if the full name isn't real *and* stripping
/// the trailing numeral yields a name that *is* real does that numeral
/// count as an observed rank.
fn split_cast_rank(name: &str) -> (&str, Option<u8>) {
    if crate::spelldata::spell_by_name(name).is_some() {
        return (name, None);
    }
    match name.rsplit_once(' ') {
        Some((base, tail)) if is_roman_numeral(tail) && crate::spelldata::spell_by_name(base).is_some() => {
            (base, roman_to_u8(tail))
        }
        _ => (name, None),
    }
}

/// Standard subtractive-notation roman numeral -> integer, clamped to
/// `u8` (real observed ranks are nowhere near 255). `is_roman_numeral`
/// already guarantees the character set; this additionally requires a
/// well-formed value, returning `None` for a charset-valid but
/// nonsensical ordering rather than a wrong number.
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
        let next = if i + 1 < bytes.len() { value(bytes[i + 1]) } else { 0 };
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
        assert_eq!(split_cast_rank("Garrison's Mighty Mana Shock IX"), ("Garrison's Mighty Mana Shock", Some(9)));
    }

    #[test]
    fn a_spell_line_variant_is_never_treated_as_a_rank() {
        // why: the exact bug reported -- "Monster Summoning" alone isn't
        // a real spell, only "Monster Summoning II"/"III" are, each its
        // own catalog entry; same for the Yaulp line.
        assert_eq!(split_cast_rank("Monster Summoning II"), ("Monster Summoning II", None));
        assert_eq!(split_cast_rank("Monster Summoning III"), ("Monster Summoning III", None));
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
        assert_eq!(split_cast_rank("Some Made Up Ability X"), ("Some Made Up Ability X", None));
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
        r.observe(2000, "Ice Comet", 6); // a lower re-observation must not regress it
        assert_eq!(r.rank_of("Ice Comet"), Some(9));
        assert_eq!(r.rank_of("Never Cast"), None);
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

/// `loot.self`'s `corpse` capture is always "<mob>'s corpse" -- confirmed
/// against every loot line in the reference log (546 lines, 0 exceptions).
/// Strips the fixed suffix to recover the mob's own display name, the same
/// name combat lines would use as `target`.
fn strip_corpse_suffix(corpse: &str) -> &str {
    corpse.strip_suffix("'s corpse").unwrap_or(corpse)
}

/// EQ's four currency denominations, most to least valuable, with their
/// copper equivalents (1 platinum = 10 gold = 100 silver = 1000 copper --
/// the classic EQ conversion; nothing found scraping this fork's own data
/// suggests EQL changed it). `Store::amount` for `EventKind::Currency` is
/// always in copper, so a gain phrased in any mix of denominations still
/// lands on one directly comparable number.
const CURRENCY_DENOMINATIONS: &[(&str, u64)] = &[
    ("platinum", 1000),
    ("gold", 100),
    ("silver", 10),
    ("copper", 1),
];

/// Reads a denomination list like "2 platinum, 5 gold, 7 silver and 2
/// copper" into one copper-equivalent total. Deliberately not a regex on
/// the whole clause, and not done in the rule pack at all: confirmed
/// against the real log, this server phrases what's structurally the same
/// 4-denomination list two different ways depending on which line it's
/// on -- comma-and-"and" joined on an auto-sell/corpse-loot ("2 platinum,
/// 5 gold, 7 silver and 2 copper"), plain space-joined on a direct vendor
/// sale ("9 platinum 5 gold 7 silver") -- so this just walks the text
/// looking for "<number> <denomination>" pairs directly, order- and
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

/// What one classified line resolves to, ahead of the sequential merge.
/// `Action` is the ordinary case. The other two mirror
/// `Ingest::flavor_evidence_for`'s own two checks, looked up here on the
/// classify thread since both are stateless (`classes_for_flavor`,
/// `third_person_flavor`) with no `Ingest` access needed -- same reasoning
/// `extract_action` already gets to run in parallel. Without this,
/// backfill would silently never see either: the parallel path used to
/// drop unmatched lines' text entirely, so only newly-arriving live lines
/// (`Ingest::route`'s own inline path) could ever feed the dictionary.
enum Classified {
    Action(Action),
    /// `text` is a known first-person landing message verbatim -- about
    /// "You". `classes` feeds the still-sequential, order-dependent Quick
    /// Buff attribution (`Ingest::attribute_flavor_hit`); `text` (owned,
    /// since it has to survive the thread boundary) feeds the
    /// unconditional state ping (`Ingest::record_effect_ping`).
    SelfFlavorHit { classes: &'static [String], text: String },
    /// `text`'s first-person reconstruction is a known landing message --
    /// about `who`, not "You". Never class evidence -- see
    /// `third_person_flavor`'s own doc.
    ThirdPersonFlavorHit { who: String, text: String },
}

/// One chunk's worth of classification, ready to be replayed sequentially.
/// `matched` keeps every matched line's timestamp even when it produced no
/// `Classified` (a "noise" rule, or unmatched text with no flavor hit),
/// because the log clock still needs to advance past it in order.
struct ChunkResult {
    counts: LineCounts,
    matched: Vec<(Millis, Option<Classified>)>,
    /// This chunk's own local shape accumulation -- one `Shaper` per
    /// thread (it holds mutable scratch state, so it can't be shared),
    /// folded into `Ingest::unmatched_shapes` by `merge_unmatched_shape`
    /// once every chunk is back on the sequential merge thread.
    ///
    /// Deliberately *uncapped* here, unlike the final accumulator --
    /// `UNMATCHED_SHAPE_CAP` exists to bound a long-lived session's
    /// memory, but one chunk is transient (built, merged, and dropped
    /// within a single `backfill_lines` call) and already bounded by its
    /// own line count, so there's nothing to cap. An earlier version
    /// capped this locally too, reasoning a single thread's own slice
    /// would never realistically hit 4096 distinct shapes on its own --
    /// wrong against the real reference log (mostly buff-landing flavor
    /// text, which combines spell x target x wording into a genuinely
    /// huge distinct-shape space): at 8 threads, individual chunks did
    /// hit their own local cap, and since each chunk dropped its overflow
    /// independently with no way to know whether the *global* map still
    /// had room for that exact shape, the merged total came out wrong
    /// both ways in turn (undercounted when drops were silent, then
    /// overcounted when they were credited to overflow unconditionally).
    /// Caught by cross-checking against the real `eqlp coverage` output,
    /// not by inspection -- fixed by removing the local cap entirely
    /// rather than layering more accounting on top of it.
    unmatched_shapes: HashMap<Vec<u8>, ShapeStat>,
}

/// Classification only -- the expensive, embarrassingly-parallel part. No
/// access to `Ingest`; a chunk is classified against nothing but the
/// (immutable, `Send + Sync`) `Engine` and its own lines, which is what
/// makes it safe to run on someone else's thread.
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
                        Some(Classified::SelfFlavorHit { classes, text: text.to_string() }),
                    ));
                    true
                } else if let Some((who, canonical)) = third_person_flavor(&text) {
                    let ts_ms = ts.secs() * 1000;
                    matched.push((
                        ts_ms,
                        Some(Classified::ThirdPersonFlavorHit { who, text: canonical }),
                    ));
                    true
                } else if let Some((who, canonical)) = verb_conjugated_flavor(&text) {
                    let ts_ms = ts.secs() * 1000;
                    matched.push((
                        ts_ms,
                        Some(Classified::ThirdPersonFlavorHit { who, text: canonical }),
                    ));
                    true
                } else {
                    false
                };
                // A recognized line is understood, just not by a rule
                // pattern -- it has no business in the Debug module's
                // "Unparsed" shape list. Only a real miss on both checks
                // still gets shape-clustered -- see `Ingest::route`'s
                // matching comment.
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

/// Splits `raw` into complete lines, CRLF-tolerant, holding back a trailing
/// line with no terminating `\n` -- the game may still be mid-write of it.
/// Same contract as `eqlp_core::frame::Framer` for a single buffer, just
/// without needing a streaming callback (see `backfill_lines`). `tail_worker.rs`
/// frames a backfill's raw bytes once up front and hands `backfill_lines`
/// smaller pieces of the result -- see that function's doc for why. Fully
/// `pub`, not `pub(crate)`, so `examples/dump_fixtures.rs` (compiled as its
/// own crate against this one's lib target) can frame `fixtures/reference-
/// slice.log` the identical way a real backfill would.
pub fn framed_lines(raw: &[u8]) -> Vec<&[u8]> {
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

/// Parses `lines` (a file's history, or a bounded piece of one) across
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
/// Takes already-framed `lines` rather than a raw buffer -- framing
/// (`framed_lines`) is a cheap single-threaded linear scan, not the
/// bottleneck, so it isn't repeated here; the point of taking a slice
/// rather than owning the framing is so `tail_worker.rs`'s backfill loop
/// can frame a whole file once and then hand this function one bounded
/// chunk at a time. That split exists because `Ingest`'s lock would
/// otherwise be held for a multi-million-line file's *entire* replay, with
/// the UI thread's very first `get_status` blocked on it and no progress
/// tick emitted until it was already done -- the app would sit on a blank
/// window, then jump straight to "fully caught up" with no number ever
/// counting up in between. Called once per chunk instead, each call is a
/// self-contained unit of work: the caller re-acquires the lock and emits
/// a tick between calls.
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

    // Sequential merge, in file order (chunks were split contiguously, so
    // iterating results in order is iterating the file in order): this is
    // the part that can't parallelise, but it's hashmap/vec work with no
    // regex left in it.
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
                None => {}
            }
        }
    }
}

#[cfg(test)]
mod xp_tests {
    use super::*;
    use crate::parser::build_engine;

    /// Real lines from `eqlog_Manipulator_rivervale.txt` (2026-07-28,
    /// 15:02:08-15:02:57), unedited -- a solo pet kill with a
    /// "You gain experience! (11.000%)" line sitting between the kill's
    /// last two damage lines and its own "You have slain a fragile pet!".
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

    /// Real lines from the same log (15:03:55-15:03:58): a quest turn-in
    /// firing the identical "You gain experience!" line, with no kill
    /// anywhere nearby -- see `Ingest::record_xp`'s doc.
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
        // Not asserting `e.slain` here -- that flag only flips once
        // `drain_closed` processes the encounter off `self.encounters.
        // closed`, which needs more trailing quiet time than this short
        // snippet gives it. `enc` resolving to the *right* encounter
        // (matched by name, found via `encounter_id_for_victim` right as
        // the death itself is recorded) is what this test is actually
        // checking -- the fact of attribution, not this encounter
        // tracker's own separate close-out timing.
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

    /// Real lines, eqlog_Manipulator_rivervale.txt (Aug 16 -- the same
    /// file's later "no AFK anywhere near the file's own start" span,
    /// confirming `session_start` prefers the AFK-off timestamp over
    /// `first_ts` once one exists).
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
        // 2026-08-16 19:43:03 UTC, well after the file's own 2026-07-28
        // start -- proves session_start picked the AFK-off line, not
        // first_ts.
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
        // No afk.off seen yet -- session_start falls back to first_ts,
        // not left `None` or advanced to the afk.on line itself.
        assert_eq!(ing.session_start(), ing.first_ts);
    }
}

#[cfg(test)]
mod aa_tests {
    use super::*;
    use crate::parser::build_engine;

    /// Real lines, eqlog_Manipulator_rivervale.txt -- a first-ever
    /// purchase (rank 1, "gained the ability"), a later rank-up ("improved
    /// ... N"), a free (0-cost) first rank, and a singular-"point" rank-up
    /// (cost exactly 1) -- the one grammar wrinkle that distinguishes
    /// aa.gained (always plural "points", even at cost 0 or 1) from
    /// aa.improved (singular "point" at exactly cost 1). Also includes an
    /// innate-skill-grant line ("gained the ability to use ..."), which
    /// must NOT be picked up as an AA.
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
        // 4 real AA lines went in; "gained the ability to use Double
        // Attack." (no quotes, no cost clause) must not have added a 5th.
        assert_eq!(grants.len(), 4);

        assert_eq!(grants[0].name, "Spell Casting Deftness");
        assert_eq!(grants[0].rank, 1); // aa.gained always synthesizes rank 1
        assert_eq!(grants[0].cost, 2);

        assert_eq!(grants[1].name, "Unbound Drain");
        assert_eq!(grants[1].cost, 0); // a free first rank is still a real grant

        assert_eq!(grants[2].name, "Spell Casting Deftness");
        assert_eq!(grants[2].rank, 2); // aa.improved's own rank digit, not synthesized
        assert_eq!(grants[2].cost, 4);

        assert_eq!(grants[3].name, "Innate Regeneration");
        assert_eq!(grants[3].rank, 2);
        assert_eq!(grants[3].cost, 1); // singular "1 ability point." still parses

        assert_eq!(ing.aa.total_spent(), 2 + 0 + 4 + 1);
    }
}

#[cfg(test)]
mod exaltation_proc_tests {
    use super::*;
    use crate::parser::build_engine;

    /// Real lines, eqlog_Manipulator_rivervale.txt -- two items each
    /// firing more than once, in the order they actually appear. A
    /// non-Exaltation-labeled "Your X (Y) Z." line has no real example in
    /// the reference log (see ExaltationProcs' own doc), so this only
    /// pins the Exaltation case, which is what actually matters.
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
        assert_eq!(ing.exaltation_procs.count("Black Tome with Silver Runes"), 1);
        assert_eq!(ing.exaltation_procs.count("Something Never Seen"), 0);
    }

    /// Unit-level, not through the parser -- `observe`'s own contract:
    /// the *first* call's timestamp sticks, a later repeat only bumps the
    /// count, never overwrites when the socket was first confirmed live.
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

    /// Real lines, eqlog_Manipulator_rivervale.txt -- "Color Flux"
    /// memorized twice (gem-swap re-memorize, common during real play),
    /// "Suffocating Sphere" memorized once, and a "Beginning to
    /// memorize"/"You forget" pair that must NOT register as known --
    /// only a *completed* memorize is proof.
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
        // Color Flux + Suffocating Sphere -- Ice Spear only ever began
        // memorizing, never finished, so it must not appear as Known.
        assert_eq!(known.len(), 2);
        assert!(known.contains_key("Suffocating Sphere"));

        // Ice Spear is the Possible tier's whole reason to exist: a real
        // "Beginning to memorize" with no matching finish anywhere in the
        // log. Must not also show up in known().
        let possible: std::collections::HashMap<&str, Millis> = ing.spellbook.possible().collect();
        assert_eq!(possible.len(), 1);
        assert!(possible.contains_key("Ice Spear"));
        assert!(!known.contains_key("Ice Spear"));

        // Color Flux completed twice (17:10:46, then again at 18:00:12
        // after being forgotten and re-memorized) -- first_seen must stay
        // pinned to the *first* completion, confirmed here against a
        // second Ingest that only ever sees that first line.
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

    /// Real lines, eqlog_Manipulator_rivervale.txt -- scribing a brand
    /// new scroll is now the primary "added to spellbook" signal (596/593
    /// real begin/finish occurrences), not just a fallback via memorize.
    /// "Levitate" scribes clean; "Pillar of Fire" only ever begins here
    /// (its own real finish line lands after, but this excerpt cuts it
    /// off on purpose) to prove scribe has its own Possible tier too, not
    /// just memorize.
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

    /// A spell that began via one channel (memorize) and finished via the
    /// *other* (scribe) -- both prove the same fact (spellbook
    /// membership), so this must still reach Known, and `first_began`
    /// must stay pinned to the memorize attempt, not slide to the later
    /// scribe.
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

    /// Throwaway: sanity-checks Known/Possible counts against the full
    /// 2.27M-line reference log. Not a permanent test (depends on a file
    /// path that only exists on this machine) -- just confirms the real
    /// numbers are sane (mostly Known, a small Possible tail) before
    /// shipping, the same discipline `unmatched_shape_tests`' own
    /// throwaway cross-check used.
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

    /// Real lines, eqlog_Manipulator_rivervale.txt: "<Name>'s hand is
    /// covered with a dull aura." appears under two different real
    /// casters (genuinely unrecognized -- not a `spell_flavor.json` key
    /// under either its own text or `third_person_flavor`'s
    /// reconstruction, confirmed against the real log), which must
    /// collapse to one shared shape; "The jig sends..." and "<Name>'s
    /// voice booms." are both real *flavor-recognized* lines (self and
    /// third-person respectively -- see `effect_ping_tests`), which must
    /// never show up here at all now, alongside one real rule-*matched*
    /// line, unaffected either way. `threads: 4` on a 6-line input forces
    /// multiple single-line chunks, exercising `classify_chunk`'s
    /// per-thread accumulation and `merge_unmatched_shape`'s merge, not
    /// just the single-chunk case.
    const LINES: &str = "\
[Tue Jul 28 15:02:14 2026] Xscyte's hand is covered with a dull aura.
[Tue Jul 28 15:02:15 2026] Harli's hand is covered with a dull aura.
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

        // Still counts every rule-engine miss, flavor-recognized or not --
        // this is "did a *pattern* match", a separate question from "does
        // the app understand this line at all". See `Ingest::route`'s
        // matching comment.
        assert_eq!(ing.counts.unmatched, 5); // dull-aura x2 + jig + voice-booms x2
        assert_eq!(ing.counts.matched, 1); // death.you_slew
        assert_eq!(
            ing.unmatched_shapes_distinct(),
            1,
            "only the genuinely-unrecognized dull-aura pair should cluster here"
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
            examples.iter().any(|e| e.contains("dull aura")),
            "the genuinely-unrecognized pair should be kept as the example"
        );
        assert!(
            !examples.iter().any(|e| e.contains("jig") || e.ends_with("voice booms.")),
            "flavor-recognized lines are understood -- they must not appear as unparsed"
        );
        assert!(
            !examples.iter().any(|e| e.contains("slain")),
            "the matched death line must never appear as an unmatched shape"
        );
    }

    /// Same exclusion, exercised through `Ingest::route`'s own inline
    /// path (live tail), not `backfill_lines` -- a separate code path
    /// with its own copy of this logic, so it needs its own real-line
    /// check rather than trusting the backfill test to cover both.
    #[test]
    fn a_flavor_recognized_line_is_excluded_from_unparsed_on_the_live_path_too() {
        let engine = build_engine().expect("pack builds");
        let mut matcher = engine.matcher();
        let mut ing = Ingest::default();
        for line in [
            &b"[Tue Jul 28 15:02:14 2026] Xscyte's hand is covered with a dull aura."[..],
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
            "only the genuinely-unrecognized dull-aura line should show up"
        );
        let top = ing.unmatched_shapes_top(10);
        assert!(top.iter().any(|(_, s)| String::from_utf8_lossy(&s.example).contains("dull aura")));
        assert!(
            !top.iter().any(|(_, s)| {
                let e = String::from_utf8_lossy(&s.example);
                e.contains("jig") || e.ends_with("voice booms.")
            }),
            "flavor-recognized lines must not appear as unparsed on the live path either"
        );
    }

    /// Throwaway: cross-checks the real numbers `eqlp coverage --pack
    /// packs/eql.toml <log> --top 5` already printed against the full
    /// 2.27M-line reference log (4096 distinct shapes, 177354-line
    /// overflow, "The jig..." at count 7824) -- not a permanent test
    /// (depends on a file path that only exists on this machine).
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

        // The real invariant to hold, not an exact match against `eqlp
        // coverage`'s own single-threaded output: which specific shapes
        // land in the last few slots before the 4096 cap fills is
        // sensitive to merge order (a chunk's HashMap iteration order
        // isn't file order), so the *overflow count* genuinely isn't
        // reproducible bit-for-bit across a parallel merge -- confirmed
        // directly (it came out different on three different accounting
        // strategies here, none of them matching the CLI's 177354).
        // What must always hold regardless of ordering: every unmatched
        // line is accounted for exactly once, either inside a tracked
        // shape's count or in the overflow tally -- nothing silently
        // vanishes and nothing gets double-counted.
        let tracked_total: u64 = ing
            .unmatched_shapes_top(usize::MAX)
            .iter()
            .map(|(_, s)| s.count)
            .sum();
        assert_eq!(
            tracked_total + ing.unmatched_shapes_overflow(),
            ing.counts.unmatched
        );

        // Order-INsensitive facts, which any correct accumulation must
        // get right regardless of merge order -- these matched `eqlp
        // coverage --top 5`'s own real output exactly:
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

    /// `notifications::notification_for`'s own tests check the pure
    /// rule-id -> message mapping; this checks the actual wiring
    /// `tail_worker.rs` depends on -- that `Ingest::route`, run live
    /// (`mark_live`), really does push into `pending_notifications`, and
    /// that backfill (not live) does *not* -- a freshly-launched app
    /// replaying days of history must not fire a burst of sounds for
    /// things that already happened.
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

    /// Each real line pulled from the unmatched-shape backlog now matches
    /// its own rule, not the catch-all "unmatched" bucket.
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

    /// The actual point of the mitigation-flag design: a missed, blocked,
    /// dodged, or parried swing lands on the *same* ability row a landed
    /// one of that type would ("Punch", "Slash"), tagged by which kind of
    /// avoidance it was -- not a separate synthetic "Miss"/"Block"/
    /// "Dodge"/"Parry" ability. `attempts()` (landed + all four avoidance
    /// counts) is the honest denominator for a real hit rate.
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

        let refuse = ing.store.names.get("Refuse").expect("Refuse should be interned");
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

        let skeleton = ing.store.names.get("Ice boned skeleton").expect("skeleton should be interned");
        let skel_rows = by_ability(&ing.store, &Filter::default().by(skeleton));
        let skel_punch = skel_rows
            .iter()
            .find(|r| ing.store.ability_name(r.ability) == "Punch")
            .expect("a Punch row should exist");
        assert_eq!(skel_punch.dodged, 1);
        assert_eq!(skel_punch.hits, 0, "never actually landed");

        let snake = ing.store.names.get("a rattlesnake").expect("rattlesnake should be interned");
        let snake_rows = by_ability(&ing.store, &Filter::default().by(snake));
        let snake_slash = snake_rows
            .iter()
            .find(|r| ing.store.ability_name(r.ability) == "Slash")
            .expect("a Slash row should exist");
        assert_eq!(snake_slash.parried, 1);
    }

    /// The real bug this session found in the other direction: all four
    /// avoidance rules hard-anchored `!$` with nothing allowed to follow,
    /// so a real flagged miss/block/dodge/parry ("(Riposte)", "(Rampage)",
    /// "(Flurry)") fell through to unmatched *entirely* -- not even
    /// counted as a plain avoidance. Real lines, all four kinds, each
    /// still lands on its own ability row (proving the swing itself
    /// wasn't lost) *and* carries the special-attack-type bit.
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

        let socho = ing.store.names.get("Socho Darkpaw").expect("Socho Darkpaw should be interned");
        let rows = by_ability(&ing.store, &Filter::default().by(socho));
        let hit = rows
            .iter()
            .find(|r| ing.store.ability_name(r.ability) == "Hit")
            .expect("a Hit row should exist");
        assert_eq!(hit.missed, 1);
        assert_eq!(hit.blocked, 1);
        assert_eq!(hit.dodged, 1);
        assert_eq!(hit.parried, 1);
        assert_eq!(hit.attempts(), 4, "none of the 4 flagged swings should be lost");

        // Confirm the flag itself, not just that the swing landed on a
        // row -- walk the raw store rows for the four Miss-kind events.
        let miss_flags: Vec<eqlp_store::Flags> = (0..ing.store.kind.len())
            .filter(|&i| ing.store.kind[i] == eqlp_store::EventKind::Miss && ing.store.actor[i] == socho)
            .map(|i| ing.store.flags[i])
            .collect();
        assert_eq!(miss_flags.len(), 4);
        assert!(miss_flags[0] & eqlp_store::flag::RIPOSTE != 0, "{:#x}", miss_flags[0]);
        assert!(miss_flags[1] & eqlp_store::flag::RAMPAGE != 0, "{:#x}", miss_flags[1]);
        assert!(miss_flags[2] & eqlp_store::flag::FLURRY != 0, "{:#x}", miss_flags[2]);
        assert!(miss_flags[3] & eqlp_store::flag::RIPOSTE != 0, "{:#x}", miss_flags[3]);
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

    /// A stance's own class list feeds `classdetect` the exact same way an
    /// unambiguous spell already does -- two real "assume a berserker
    /// stance" lines, on two distinct zone visits (`MIN_UNAMBIGUOUS_CASTS`
    /// counts *visits*, not raw occurrences -- see its own doc), confirm
    /// Berserker with no spell cast involved at all.
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
        let configured = ing.classes.configuration_of_visit(you.0, ing.zone.index_at(ing.now_ms()));
        assert!(configured.contains(&"Berserker".to_string()), "{configured:?}");
    }

    /// The mirror case: one occurrence isn't enough evidence yet, same bar
    /// a spell is held to.
    #[test]
    fn a_single_stance_line_is_not_enough_evidence_yet() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![b"[Tue Jul 28 15:01:00 2026] You assume a berserker stance."];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let you = ing.store.names.get("You").expect("You should be interned");
        let configured = ing.classes.configuration_of_visit(you.0, ing.zone.index_at(ing.now_ms()));
        assert!(!configured.contains(&"Berserker".to_string()), "{configured:?}");
    }
}

#[cfg(test)]
mod skill_evidence_tests {
    use super::*;
    use crate::parser::build_engine;

    /// Real shape from a real character's log: "You have become better at
    /// Tracking! (N)" was already parsed by this pack (`skill.up`) but
    /// never routed anywhere -- confirms it now reaches `classdetect` the
    /// same way an unambiguous spell would (Tracking is Bard/Druid/Ranger
    /// only, no other route -- see `skilldata`'s own doc).
    #[test]
    fn a_tracking_skill_up_narrows_the_open_slot() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        // Enchanter and Wizard proven and reinforced this visit, same as
        // the real session this reproduces; then an ambiguous cast pool
        // that only shares Ranger with Tracking's own pool.
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
            // Cure Poison: {Beastlord, Cleric, Druid, Paladin, Ranger, Shaman}.
            b"[Tue Jul 28 15:03:03 2026] You begin casting Cure Poison.",
            // Evasive stance: {Bard, Monk, Ranger, Beastlord, Rogue} -- with
            // Cure Poison's pool, narrows to {Beastlord, Ranger}.
            b"[Tue Jul 28 15:03:04 2026] You assume an evasive stance.",
            // Tracking: {Bard, Druid, Ranger} -- the real evidence this
            // whole test exists for; only Ranger survives all three pools.
            b"[Tue Jul 28 15:03:05 2026] You have become better at Tracking! (1)",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let you = ing.store.names.get("You").expect("You should be interned");
        let configured = ing.classes.configuration_of_visit(you.0, ing.zone.index_at(ing.now_ms()));
        assert_eq!(
            configured,
            vec!["Enchanter".to_string(), "Ranger".to_string(), "Wizard".to_string()],
            "{configured:?}"
        );
    }
}

#[cfg(test)]
mod invocation_evidence_tests {
    use super::*;
    use crate::parser::build_engine;

    /// Real multi-source combination: an invocation, a stance, and a
    /// skill-up, none individually enough, narrow the same open slot down
    /// together the same way three ambiguous spells would. Deliberately
    /// avoids any pool that overlaps an already-confirmed class -- one
    /// that does is read as reinforcement of the confirmed class, not
    /// evidence about the *other* candidate it also lists (correctly
    /// conservative; see `apply_pool`'s own doc).
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
            // Spellblade: {Beastlord, Paladin, Ranger, Shadow Knight}.
            b"[Tue Jul 28 15:03:03 2026] You begin reciting the spellblade invocation.",
            // Evasive: {Bard, Monk, Ranger, Beastlord, Rogue} -- combined,
            // narrows to {Beastlord, Ranger}.
            b"[Tue Jul 28 15:03:04 2026] You assume an evasive stance.",
            // Tracking: {Bard, Druid, Ranger} -- only Ranger survives all
            // three pools.
            b"[Tue Jul 28 15:03:05 2026] You have become better at Tracking! (1)",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let you = ing.store.names.get("You").expect("You should be interned");
        let configured = ing.classes.configuration_of_visit(you.0, ing.zone.index_at(ing.now_ms()));
        assert_eq!(
            configured,
            vec!["Enchanter".to_string(), "Ranger".to_string(), "Wizard".to_string()],
            "{configured:?}"
        );
    }
}

#[cfg(test)]
mod effect_ping_tests {
    use super::*;
    use crate::parser::build_engine;

    /// The correction that started this: a recognized buff-landing line is
    /// real evidence of what's true about "You" whether or not a Quick Buff
    /// window is open (an ally could have cast it) -- so the ping must not
    /// depend on the window the way class-evidence attribution does. Real
    /// line, no activation anywhere nearby.
    #[test]
    fn a_flavor_line_pings_state_with_no_quickbuff_window_open_at_all() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> =
            vec![b"[Sat Aug 08 00:01:02 2026] A burst of strength surges through your body."];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let you = ing.store.names.get("You").expect("You should be interned");
        let recent = ing.effects.recent(you.0, ing.now_ms(), 1_000);
        assert_eq!(recent, vec!["A burst of strength surges through your body."]);
    }

    /// The other half of the same correction: recognizing the line as state
    /// does *not* imply it's safe to use as class evidence. Same
    /// unambiguous (Necromancer-only) flavor text, landing across two
    /// distinct zone visits with no Quick Buff activation anywhere nearby
    /// -- the ping still fires each time, but nothing gets confirmed.
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
        let configured = ing.classes.configuration_of_visit(you.0, ing.zone.index_at(ing.now_ms()));
        assert!(configured.is_empty(), "{configured:?}");

        // But the ping itself still landed both times.
        assert_eq!(
            ing.effects.recent(you.0, ing.now_ms(), 1_000),
            vec!["A blast of acid eats at your skin."]
        );
    }

    /// Same unambiguous text, same two visits -- this time inside an open
    /// Quick Buff window each time, matching the pre-existing attribution
    /// behavior exactly (unchanged by this split): two distinct-visit
    /// unambiguous sightings confirm the class.
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
            // Past PULSE_WINDOW_MS -- flushes the pending evidence above
            // via record_effect_ping's own staleness commit (see its
            // doc). Nothing else advances the clock in a short synthetic
            // test the way 72,000+ real pings would. A *different* real
            // flavor line on purpose -- reusing the same text here would
            // itself look like the exact pulsing pattern this whole
            // mechanism now exists to catch.
            b"[Tue Jul 28 15:02:20 2026] A burst of strength surges through your body.",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let you = ing.store.names.get("You").expect("You should be interned");
        let configured = ing.classes.configuration_of_visit(you.0, ing.zone.index_at(ing.now_ms()));
        assert!(
            configured.contains(&"Necromancer".to_string()),
            "{configured:?}"
        );
    }

    /// `fight_state_at`'s own DTO field, exercised end to end through a
    /// real encounter: an ally's buff lands on "You" mid-fight (no
    /// activation nearby -- just a real ally-cast effect), then a scrub
    /// query for a moment shortly after shows it as a recent effect.
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
        assert_eq!(
            you_state.recent_effects,
            vec!["A burst of strength surges through your body.".to_string()]
        );
    }

    /// The Amplification correction: a third-person landing line is real
    /// evidence of state on *whoever it landed on*, not "You" -- real
    /// line, real repeat cadence (a bard song's own pulse), pulled
    /// straight from the reference log.
    #[test]
    fn a_third_person_landing_pings_state_on_the_actual_target_not_you() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Fri Aug 07 16:30:31 2026] Handstuff's voice booms.",
            b"[Fri Aug 07 16:30:37 2026] Handstuff's voice booms.",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let handstuff = ing.store.names.get("Handstuff").expect("Handstuff should be interned");
        assert_eq!(
            ing.effects.recent(handstuff.0, ing.now_ms(), 60_000),
            vec!["Your voice booms.", "Your voice booms."],
            "canonical first-person text, not the raw third-person line"
        );

        let you = ing.store.names.get("You");
        assert!(
            you.is_none_or(|s| ing.effects.recent(s.0, ing.now_ms(), 60_000).is_empty()),
            "must not land on You -- it landed on Handstuff"
        );
    }

    /// Never class evidence, even if it happens to fall inside an open
    /// Quick Buff window -- a third-person line doesn't prove an *ally*
    /// cast it, let alone the log owner (a mob's own DoT ticking on
    /// someone lands the exact same way).
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

        // "You" never even needs to be interned as an entity here -- every
        // line in this test is third-person, so if class evidence leaked
        // through, `known_entities` would be the first place it showed up.
        assert!(
            ing.classes.known_entities().next().is_none(),
            "no evidence should ever be attributed to anyone from these lines"
        );
    }

    /// A real possessive-shaped line whose first-person reconstruction is
    /// *not* a known landing message (a fully unrelated buff this app has
    /// no scraped text for, per the reference-log audit) produces no ping
    /// at all -- the dictionary gate is what keeps this transform from
    /// manufacturing false positives out of ordinary possessive sentences.
    #[test]
    fn a_possessive_line_that_does_not_reconstruct_a_known_message_pings_nothing() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> =
            vec![b"[Fri Aug 07 16:30:31 2026] Bravesirrobin's hand is covered with a dull aura."];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let bravesirrobin = ing.store.names.get("Bravesirrobin");
        assert!(
            bravesirrobin.is_none_or(|s| ing.effects.recent(s.0, ing.now_ms(), 60_000).is_empty())
        );
    }

    /// The conjugated (non-possessive) third-person form: "@ feels much
    /// faster." from the user's own report. Real line, real regular verb
    /// conjugation ("feel" -> "feels").
    #[test]
    fn a_conjugated_third_person_landing_pings_state_on_the_actual_target() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> =
            vec![b"[Fri Aug 07 16:30:31 2026] Draxiz N`Ryt feels much faster."];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let draxiz = ing.store.names.get("Draxiz N`Ryt").expect("Draxiz N`Ryt should be interned");
        assert_eq!(
            ing.effects.recent(draxiz.0, ing.now_ms(), 60_000),
            vec!["You feel much faster."]
        );
    }

    /// A real *multi-word* entity name ("The Prophet"), proving the "try
    /// every space as a possible split point" approach in
    /// `verb_conjugated_flavor` finds the right boundary regardless of how
    /// many words the name itself has -- and the irregular "are" -> "is"
    /// conjugation, both from real log lines.
    #[test]
    fn a_multi_word_name_still_resolves_the_correct_split_point() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Fri Aug 07 22:03:04 2026] The Prophet is struck by a sudden force.",
            b"[Fri Aug 07 22:03:09 2026] The Prophet is struck by a sudden burst of force.",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let prophet = ing.store.names.get("The Prophet").expect("The Prophet should be interned");
        assert_eq!(
            ing.effects.recent(prophet.0, ing.now_ms(), 60_000),
            vec![
                "You are struck by a sudden force.",
                "You are struck by a sudden burst of force.",
            ]
        );
    }

    /// A real "Your <noun> <verb> ..." recovery: "adheres to the ground."
    /// reconstructs "Your feet adhere to the ground.", not "You adhere...".
    #[test]
    fn a_your_noun_verb_shape_recovers_its_own_real_key() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![b"[Fri Aug 07 16:30:31 2026] Akkirus adheres to the ground."];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let akkirus = ing.store.names.get("Akkirus").expect("Akkirus should be interned");
        assert_eq!(
            ing.effects.recent(akkirus.0, ing.now_ms(), 60_000),
            vec!["Your feet adhere to the ground."]
        );
    }

    /// "@ combusts." is the one confirmed-by-hand exception
    /// (`THIRD_PERSON_VERB_ALIASES`) -- same spell as "You feel your skin
    /// combust.", just a shortened third-person announcement rather than
    /// a conjugated one, per the user's own correction.
    #[test]
    fn the_named_combust_alias_pings_the_canonical_first_person_text() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![b"[Fri Aug 07 16:30:31 2026] Baron Telyx V`Zher combusts."];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let baron = ing.store.names.get("Baron Telyx V`Zher").expect("Baron Telyx V`Zher should be interned");
        assert_eq!(
            ing.effects.recent(baron.0, ing.now_ms(), 60_000),
            vec!["You feel your skin combust."]
        );
    }

    /// A real, still-genuine gap: no first-person text for this exists
    /// anywhere in `spell_flavor.json` at all, so nothing can reconstruct
    /// it -- correctly pings nothing rather than guessing.
    #[test]
    fn a_line_with_no_source_text_at_all_pings_nothing() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> =
            vec![b"[Fri Aug 07 16:30:31 2026] Lenekab is surrounded by a brief lupine aura."];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let lenekab = ing.store.names.get("Lenekab");
        assert!(lenekab.is_none_or(|s| ing.effects.recent(s.0, ing.now_ms(), 60_000).is_empty()));
    }

    /// The noun-keeping sibling of the plain verb-conjugation rule: "You
    /// feel your body pulse with energy." -> "'s body pulses with
    /// energy.", contrasting `combust` above (same "You feel your <noun>
    /// <verb>" shape, but that one *drops* the noun in third person while
    /// this one keeps it -- confirmed against real, distinct data for
    /// each, not assumed to generalize one way).
    #[test]
    fn a_feel_your_noun_verb_line_keeps_the_noun_in_third_person() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> =
            vec![b"[Fri Aug 07 16:30:31 2026] orc legionnaire's body pulses with energy."];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let orc = ing.store.names.get("orc legionnaire").expect("orc legionnaire should be interned");
        assert_eq!(
            ing.effects.recent(orc.0, ing.now_ms(), 60_000),
            vec!["You feel your body pulse with energy."]
        );
    }

    /// The trailing "you." -> "them." sibling: when the effect lands on
    /// someone else, the sentence's own trailing pronoun swaps too, not
    /// just the subject.
    #[test]
    fn a_trailing_you_swaps_to_them_when_it_lands_on_someone_else() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> =
            vec![b"[Fri Aug 07 16:30:31 2026] Dreadmoon feels the favor of the gods upon them."];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let dreadmoon = ing.store.names.get("Dreadmoon").expect("Dreadmoon should be interned");
        assert_eq!(
            ing.effects.recent(dreadmoon.0, ing.now_ms(), 60_000),
            vec!["You feel the favor of the gods upon you."]
        );
    }

    /// The "feel ADJ" -> "looks ADJ" family: a whole set of single-
    /// adjective buffs render as how the target visibly *looks* to
    /// onlookers rather than a conjugated "feels ADJ.".
    #[test]
    fn a_single_adjective_buff_recognizes_its_looks_form() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![b"[Fri Aug 07 16:30:31 2026] Draxiz N`Ryt looks dexterous."];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let draxiz = ing.store.names.get("Draxiz N`Ryt").expect("Draxiz N`Ryt should be interned");
        assert_eq!(
            ing.effects.recent(draxiz.0, ing.now_ms(), 60_000),
            vec!["You feel dexterous."]
        );
    }

    /// `cast.blocked`: a real spell name in "Your <spell> spell did not
    /// take hold..." is definite, first-person class evidence -- proven
    /// end to end via two distinct real zone visits, the same
    /// unambiguous-confirmation path a landed cast would use.
    #[test]
    fn a_blocked_cast_still_confirms_class_evidence_from_its_own_spell_name() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Tue Jul 28 15:01:00 2026] You have entered Befallen.",
            b"[Fri Aug 14 21:11:25 2026] Your Berserker Strength spell did not take hold on Hakujin. (Blocked by Berserker Spirit.)",
            b"[Tue Jul 28 15:02:00 2026] You have entered West Karana.",
            b"[Fri Aug 14 21:11:25 2026] Your Berserker Strength spell did not take hold on Joneker. (Blocked by Berserker Spirit.)",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);

        // "Berserker Strength" is an Enchanter spell in this data despite
        // its name -- confirmed directly against packs/spell_classes.json,
        // not assumed from the name.
        let you = ing.store.names.get("You").expect("You should be interned");
        let configured = ing.classes.configuration_of_visit(you.0, ing.zone.index_at(ing.now_ms()));
        assert!(
            configured.contains(&"Enchanter".to_string()),
            "{configured:?}"
        );
    }

    /// The `blocker` half: names a buff already active on the *target*,
    /// not the caster -- real state, fed to `Effects` the same as any
    /// other recognized fact.
    #[test]
    fn a_blocked_cast_pings_the_blocker_as_state_on_the_target() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Fri Aug 14 21:11:25 2026] Your Boon of the Clear Mind spell did not take hold on Joneker. (Blocked by Clarity.)",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let joneker = ing.store.names.get("Joneker").expect("Joneker should be interned");
        assert_eq!(ing.effects.recent(joneker.0, ing.now_ms(), 60_000), vec!["Clarity"]);
    }

    /// The real minority with no trailing parenthetical at all -- class
    /// evidence still lands, but there's nothing to ping.
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
        assert!(
            bravesirrobin.is_none_or(|s| ing.effects.recent(s.0, ing.now_ms(), 60_000).is_empty())
        );
    }

    /// `dot.damage_from_you`: a real DoT tick previously fell through both
    /// existing damage-from rules entirely (neither `dot.damage`'s "by
    /// <caster>" clause nor `dot.damage_uncredited`'s no-caster shape
    /// matches "damage from your ..."). Now a real `Damage` row,
    /// attributed to "You".
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

    /// Poison/disease were already matched (kind "state") but produced no
    /// `Action` at all before `StateEffect` existed to feed -- now real
    /// pings, self and third-party both, same as any other recognized
    /// state fact.
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
            ing.effects.recent(you.0, ing.now_ms(), 60_000),
            vec!["Diseased", "Poisoned"]
        );
        let dojii = ing.store.names.get("Dojii").expect("Dojii should be interned");
        assert_eq!(ing.effects.recent(dojii.0, ing.now_ms(), 60_000), vec!["Diseased"]);
        let snake = ing.store.names.get("a rattlesnake").expect("rattlesnake should be interned");
        assert_eq!(ing.effects.recent(snake.0, ing.now_ms(), 60_000), vec!["Poisoned"]);
    }

    /// The named `yaulp` alias: same effect as the scraped first-person
    /// text, just reworded rather than conjugated.
    #[test]
    fn the_named_yaulp_alias_pings_the_canonical_first_person_text() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![b"[Fri Aug 07 16:33:09 2026] Flewdur lets loose a mighty yaulp."];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let flewdur = ing.store.names.get("Flewdur").expect("Flewdur should be interned");
        assert_eq!(
            ing.effects.recent(flewdur.0, ing.now_ms(), 60_000),
            vec!["You feel a surge of strength as you let forth a mighty yaulp."]
        );
    }

    /// The "feel X" -> "is X" family (multi-word tail, not just a single
    /// adjective) -- a second real substitute-verb pattern off the same
    /// "feel" source as the "looks ADJ" family.
    #[test]
    fn a_feel_x_buff_recognizes_its_is_x_form() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![b"[Fri Aug 07 16:30:31 2026] Bravesirrobin is protected from magic."];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let bravesirrobin = ing.store.names.get("Bravesirrobin").expect("Bravesirrobin should be interned");
        assert_eq!(
            ing.effects.recent(bravesirrobin.0, ing.now_ms(), 60_000),
            vec!["You feel protected from magic."]
        );
    }

    /// `ability.activated`: real, almost-always-third-person lines are
    /// class evidence for *whoever activated it*, not "You" -- the point
    /// this whole rule exists for. Two distinct visits, same real poison,
    /// confirms Rogue for the activator, not the log owner.
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

        let aella = ing.store.names.get("Aella").expect("Aella should be interned");
        let configured = ing.classes.configuration_of_visit(aella.0, ing.zone.index_at(ing.now_ms()));
        assert!(configured.contains(&"Rogue".to_string()), "{configured:?}");

        // "Aella" herself, not "You" -- the log owner gets no evidence at
        // all from a line they were never the subject of.
        assert!(ing.store.names.get("You").is_none(), "You should never be interned by this");
    }

    /// The state-ping half: fed to `Effects` on the activator regardless
    /// of whether `classdata` recognizes the ability -- "what's now on
    /// their weapon" is real state either way.
    #[test]
    fn an_activated_ability_pings_state_on_its_activator_even_when_unrecognized() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![b"[Fri Aug 07 00:00:00 2026] Bigneum activates Skull Bash."];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let bigneum = ing.store.names.get("Bigneum").expect("Bigneum should be interned");
        assert_eq!(ing.effects.recent(bigneum.0, ing.now_ms(), 60_000), vec!["Skull Bash"]);
    }

    /// Regression guard: the general rule must never shadow Quick Buff's
    /// own dedicated rule (which opens the buff-attribution window) --
    /// confirmed by checking the *specific* real behavior only
    /// `ability.quickbuff` triggers still fires.
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
            // See the matching comment in the test above -- a different
            // real flavor line, past PULSE_WINDOW_MS, flushes the pending
            // evidence.
            b"[Tue Jul 28 15:02:20 2026] A burst of strength surges through your body.",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let you = ing.store.names.get("You").expect("You should be interned");
        let configured = ing.classes.configuration_of_visit(you.0, ing.zone.index_at(ing.now_ms()));
        assert!(configured.contains(&"Necromancer".to_string()), "{configured:?}");
    }

    /// The exact real false positive the user caught: a group-wide buff
    /// (not the player's own Quick Buff) lands on the player *and* a
    /// named ally within the same tight window as the player's own
    /// activation. Real text, real timing (3s after activation, matching
    /// the reference log) -- "Magician" must never get confirmed, because
    /// it was never the player's own Quick Buff proc.
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
        let configured = ing.classes.configuration_of_visit(you.0, ing.zone.index_at(ing.now_ms()));
        assert!(
            !configured.contains(&"Magician".to_string()),
            "a group cast on Kabanab too must never confirm Magician for the player: {configured:?}"
        );

        // The ping itself is still real and unconditional -- the player
        // really was enveloped by flame, whoever cast it. Only the class
        // *attribution* is what gets cancelled.
        assert_eq!(
            ing.effects.recent(you.0, ing.now_ms(), 60_000),
            vec!["You are enveloped by flame."]
        );
    }

    /// The positive control: the exact same mechanism, but nothing lands
    /// on anyone else -- a genuine solo Quick Buff burst still confirms
    /// class evidence once its window safely closes. Proves the fix
    /// narrows the false positive without breaking the true positive.
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
            // A different real flavor line, past PULSE_WINDOW_MS, flushes
            // the second window's pending evidence -- see the matching
            // comment on the tests above (reusing the same text here
            // would itself look like a pulse).
            b"[Tue Jul 28 15:02:20 2026] A burst of strength surges through your body.",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let you = ing.store.names.get("You").expect("You should be interned");
        let configured = ing.classes.configuration_of_visit(you.0, ing.zone.index_at(ing.now_ms()));
        assert!(configured.contains(&"Necromancer".to_string()), "{configured:?}");
    }

    /// The other real false-positive shape: a single-target ally buff
    /// maintained (pulsing) on just the player, which never lands on
    /// anyone else at all -- so the cross-entity check above can't catch
    /// it, but its own repeat cadence gives it away. Real text, real
    /// cadence (~6s, matching the reference log's own "mystic protection"
    /// pulse).
    #[test]
    fn a_pulsing_ally_buff_on_only_the_player_cancels_pending_quickbuff_evidence() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Tue Jul 28 15:01:00 2026] You have entered Befallen.",
            // Pulsing before the player ever Quick Buffs -- already
            // proof this isn't tied to Quick Buff timing at all.
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
        let configured = ing.classes.configuration_of_visit(you.0, ing.zone.index_at(ing.now_ms()));
        assert!(
            !configured.contains(&"Bard".to_string()),
            "a maintained ally song must never confirm Bard for the player: {configured:?}"
        );

        // Still real, unconditional state -- the pulse ping itself is
        // untouched, only the class attribution is cancelled.
        assert!(!ing.effects.recent(you.0, ing.now_ms(), 60_000).is_empty());
    }

    /// `state.location`: the real `/loc` line from the reference log.
    #[test]
    fn a_loc_reading_is_captured_as_last_loc() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> =
            vec![b"[Tue Aug 18 22:28:36 2026] Your Location is 216.51, -103.09, -20.19"];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let (ts, x, y, z) = ing.last_loc.expect("a /loc reading should have been captured");
        assert_eq!(ts, ing.now_ms());
        assert_eq!(x, 216.51);
        assert_eq!(y, -103.09);
        assert_eq!(z, -20.19);
    }

    /// Real reference-log sequence (13:19:46 cast -> 13:20:01 zone.enter,
    /// ~15s apart): "You begin casting Translocate: X" followed shortly by
    /// a zone.enter marks that visit as a confirmed Wizard teleport
    /// landing, with the exact wiki-sourced coordinates, for the Maps
    /// module's entrance guess.
    #[test]
    fn a_translocate_cast_followed_by_zoning_marks_the_visit_teleported() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Sat Aug 01 13:19:46 2026] You begin casting Translocate: North Karana.",
            b"[Sat Aug 01 13:20:01 2026] You have entered The Northern Plains of Karana.",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);
        let (_, landing) = ing.entered_via_teleport.expect("should be marked teleported");
        assert_eq!(landing.class, teleportdata::TeleportClass::Wizard);
        assert_eq!((landing.x, landing.y, landing.z), (-3685.0, 1209.0, -5.0));
    }

    /// "Circle of X" is a Druid-class teleport, distinguished from the
    /// Wizard's Translocate so the Maps module shows "druid circle"
    /// rather than "wizard spire" in its landing note.
    #[test]
    fn a_circle_cast_followed_by_zoning_marks_the_visit_druid_teleported() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Sat Aug 01 13:19:46 2026] You begin casting Circle of Karana.",
            b"[Sat Aug 01 13:20:01 2026] You have entered The Northern Plains of Karana.",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);
        let (_, landing) = ing.entered_via_teleport.expect("should be marked teleported");
        assert_eq!(landing.class, teleportdata::TeleportClass::Druid);
        assert_eq!((landing.x, landing.y, landing.z), (-2706.0, -1494.0, -4.0));
    }

    /// A proven ally's teleport cast counts too, not just "You" -- the
    /// group-shaped siblings (Portal/Ring, and Translocate/Circle cast on
    /// a group) land the whole group, so the caster being someone else in
    /// the group must still mark your own zone visit as teleported.
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
        let (_, landing) = ing.entered_via_teleport.expect("should be marked teleported");
        assert_eq!(landing.class, teleportdata::TeleportClass::Wizard);
    }

    /// An ordinary zone-line walk, no recent teleport cast, must not be
    /// mistaken for a spire/circle landing.
    #[test]
    fn an_ordinary_zone_change_is_not_marked_teleported() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![b"[Tue Jul 28 15:01:00 2026] You have entered Blackburrow."];
        backfill_lines(&mut ing, &engine, &lines, 1);
        assert!(ing.entered_via_teleport.is_none());
    }

    /// A teleport cast too long before the zone change (well past
    /// cast-time-plus-loading-screen) must not still be credited --
    /// otherwise an unrelated later zone-line walk that happens to follow
    /// a translocate from hours earlier would wrongly read as a landing.
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

    /// The bare "Gate" spell (no zone suffix -- returns the caster to
    /// their own bind point, not a fixed zone landmark) has no coordinate
    /// data in the wiki-confirmed pack and must never trigger the spire
    /// guess -- see `teleportdata`'s own doc.
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

    /// A `<Zone> Gate` cast, unlike the bare "Gate" above, names a real
    /// zone landmark with a wiki-confirmed landing and does count as a
    /// Wizard teleport (same as its higher-level Portal sibling).
    #[test]
    fn a_named_gate_cast_marks_the_visit_wizard_teleported() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Thu Jul 30 17:27:54 2026] You begin casting Cazic Temple Gate.",
            b"[Thu Jul 30 17:28:05 2026] You have entered Cazic Thule.",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);
        let (_, landing) = ing.entered_via_teleport.expect("should be marked teleported");
        assert_eq!(landing.class, teleportdata::TeleportClass::Wizard);
    }

    /// A "Circle of X"-shaped name that is *not* actually a teleport (a
    /// damage-shield/resist buff sharing the naming convention -- see
    /// `teleportdata`'s own doc) must not mark the visit teleported. Real,
    /// confirmed false positive the old name-shape-only heuristic had.
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

    /// An unproven stranger's teleport cast must never mark "You" as
    /// having landed via spire/circle -- only "You" or a *proven* ally
    /// (see `is_ally`) counts.
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

    /// Real, reported gap: `Origin` (a class-agnostic AA -- "transports
    /// you back to your starting city", confirmed against `~/eql/aa.json`)
    /// has no fixed, wiki-quotable destination the way every other
    /// teleport here does, so it never went through `last_teleport_cast`/
    /// `entered_via_teleport` at all. Confirmed directly against the real
    /// reference log that a successful "You begin casting Origin." really
    /// is followed by a real `zone.enter` on the same real cast-time +
    /// loading-screen cadence the other teleports already use this same
    /// window for. `learned_origin` is the parallel, *learned* mechanism
    /// this needs instead of a table lookup.
    #[test]
    fn an_origin_cast_followed_by_zoning_learns_the_landing_zone() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Tue Jul 28 15:51:23 2026] You begin casting Origin.",
            b"[Tue Jul 28 15:51:46 2026] You have entered Oggok.",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);
        let (_, zone) = ing.learned_origin.expect("should have learned an origin zone");
        assert_eq!(zone, "Oggok");
        // Origin has no fixed coordinate -- unlike Gate/Translocate, this
        // must stay `None` (the confirmed zone is enough on its own;
        // `commands::live_start_position`/`get_zone_context` compute a
        // real position from it lazily, once `base_dir` is available).
        assert!(ing.entered_via_teleport.is_none());
    }

    /// Real, confirmed pattern from the actual reference log: this
    /// character's own Origin landing genuinely changed over time (Oggok,
    /// then Neriak - Commons, then New Sebilis Expedition) -- `learned_origin`
    /// must track the *most recent* real confirmation, the same "last one
    /// wins" shape `last_teleport_cast`/`entered_via_teleport` already use,
    /// not the first one ever seen.
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
        let (_, zone) = ing.learned_origin.expect("should have learned an origin zone");
        assert_eq!(zone, "Neriak - Commons");
    }

    /// An interrupted cast with no retry and no subsequent zone change at
    /// all must not fabricate a learned landing -- there's genuinely
    /// nothing to learn from here (no `zone.enter` line exists in this
    /// log at all, so there's nothing *to* wrongly attribute). Real,
    /// stated limitation carried over unchanged from `last_teleport_cast`/
    /// `entered_via_teleport`, not solved here either: an interrupted cast
    /// followed by an *unrelated* zone-line walk within `TELEPORT_WINDOW_MS`
    /// (with no retry) would still be wrongly learned, since nothing
    /// cross-checks "interrupted" against the window -- no real case of
    /// that shape was found in the reference log for the Wizard-teleport
    /// side either, so this stays a known, honest gap, not a new promise.
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

    /// The real, documented "last one wins" self-heal: an interrupted cast
    /// immediately retried, where the retry actually lands, must learn the
    /// retry's own real zone -- confirmed real shape in the reference log
    /// (line 8841-9003, a fizzle then a successful retry a few minutes
    /// later).
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
        let (_, zone) = ing.learned_origin.expect("the retry should have learned a zone");
        assert_eq!(zone, "Oggok");
    }

    /// The real bug report this exists to fix: "The Ruins of Old Guk"
    /// (the raw log label) and "gukbottom" (the real map file's own
    /// shortname) share no text in common at all -- a plain substring
    /// guess against the map file name can never confirm this zone, which
    /// is exactly why the Maps module's "you are here" dot silently never
    /// appeared there. `zone::zone_matches` (already used to resolve a raw
    /// label to a wiki zone elsewhere in this app) must resolve this raw
    /// label to "Lower Guk", from which `who_name` and
    /// `zonedata::map_shortnames` get to "gukbottom" itself -- the full
    /// chain `commands::map_zones_for_raw_label` runs, exercised directly
    /// here (no `Ingest`/interning involved -- that dependency is exactly
    /// what the *other* real bug this test caught was about, see the doc
    /// on `map_zones_for_raw_label` itself for why it no longer goes
    /// through `Ingest`'s cache).
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
}
