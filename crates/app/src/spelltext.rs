//! why: exact spell landing/wears-off text -> the spell it names, for
//! confirming a buff/debuff cast's real outcome when no damage/heal
//! proves it -- `session::CastResolver` otherwise leaves these honestly
//! Unconfirmed forever (see its own doc). Built once from
//! `spelldata::spells()`'s own `msg_cast_on_you`/`msg_cast_on_other`/
//! `msg_wears_off` fields -- no separate generated file, always in sync
//! with the catalog it reads.
//!
//! Text shared by more than one spell is dropped entirely, same stance
//! this app takes everywhere else (zone aliasing, class detection): no
//! confident single answer, no guess. Confirmed against the real
//! catalog: ~25% of landing text is shared across genuinely unrelated
//! spells (e.g. "Someone fades away." names 40+ different teleports),
//! not just same-line rank siblings.

use crate::spelldata;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

/// why: every real placeholder token seen in `msg_cast_on_other`
/// (confirmed via a full scan of packs/spells.json) -- "Someone"
/// dominates, the rest are rarer wiki-scrape conventions for the same role
const PLACEHOLDERS: &[&str] = &["Someone", "Target", "Player", "Soandso", "Other_Player"];

pub struct SpellTextMatch {
    pub spell: &'static str,
    /// why: "You" for msg_cast_on_you/msg_wears_off (always self in this
    /// log), else the name stripped from the real line
    pub target: String,
    pub is_wearsoff: bool,
}

/// why: coarse fallback for text shared by multiple spells (so exact
/// naming has to drop it) -- "beneficial or detrimental to the target"
/// is still real, useful information even when "which exact spell"
/// isn't answerable. See `spell_type_polarity`'s own doc for the source data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectPolarity {
    Buff,
    Debuff,
}

/// why: every distinct `spell_type` value in packs/spells.json (30
/// total, confirmed via a full scan) sorted into beneficial/detrimental
/// to the target, or neither. A few real judgment calls: Damage Shield/
/// Block/Vision are beneficial (a shield, a block chance, and detect-
/// invis/ultravision are all things cast *for* the target, not against
/// it); Pet/Summon Item aren't a target-polarity effect at all (you're
/// not buffing or debuffing anyone by summoning something) and are
/// deliberately left unclassified rather than guessed either way.
fn spell_type_polarity(spell_type: &str) -> Option<EffectPolarity> {
    match spell_type {
        "Beneficial"
        | "Beneficial (Group only)"
        | "Statistic Buff"
        | "Resist Buff"
        | "Utility Beneficial"
        | "Heal"
        | "Heal Over Time"
        | "Pet Buff"
        | "Pet Heal"
        | "Haste"
        | "Cure"
        | "Movement Buff"
        | "Remove Curse"
        | "Invisibility"
        | "Buff"
        | "Proc Buff"
        | "Regen"
        | "Block"
        | "Vision"
        | "Damage Shield" => Some(EffectPolarity::Buff),
        "Detrimental"
        | "Utility Detrimental"
        | "Curse"
        | "Slow"
        | "Stun"
        | "Root"
        | "Statistic Debuff"
        | "Direct Damage"
        | "Damage Over Time"
        | "DD" => Some(EffectPolarity::Debuff),
        _ => None,
    }
}

/// why: accumulates every candidate spell's polarity for one text key --
/// `One` only survives if every real candidate agrees; a single
/// disagreement (or no candidate with a classifiable spell_type at all)
/// drops the key entirely, same "no confident answer, no guess" stance
/// `insert_unique` already takes for exact names.
enum PolarityAgg {
    Unknown,
    One(EffectPolarity),
    Mixed,
}

impl PolarityAgg {
    fn add(self, p: EffectPolarity) -> Self {
        match self {
            PolarityAgg::Unknown => PolarityAgg::One(p),
            PolarityAgg::One(existing) if existing == p => PolarityAgg::One(existing),
            _ => PolarityAgg::Mixed,
        }
    }
}

fn finalize_polarity(agg: HashMap<&str, PolarityAgg>) -> HashMap<&str, EffectPolarity> {
    agg.into_iter()
        .filter_map(|(k, v)| match v {
            PolarityAgg::One(p) => Some((k, p)),
            _ => None,
        })
        .collect()
}

pub struct EffectPolarityMatch {
    pub polarity: EffectPolarity,
    pub target: String,
    pub is_wearsoff: bool,
}

struct Dict {
    /// exact full-line text (msg_cast_on_you, msg_wears_off) -> (spell, is_wearsoff)
    self_text: HashMap<&'static str, (&'static str, bool)>,
    /// msg_cast_on_other's own text, its placeholder stripped -> spell.
    /// Three real shapes collapse to this one map (confirmed against the
    /// real log): "Someone's <tail>" / "'s <tail>" (possessive, matched
    /// by finding "'s " in the real line) and "Someone <tail>" / "<tail>"
    /// alone (matched by trying every space split) -- placeholder-less
    /// entries are a real, confirmed scrape shape, not a bug.
    other_tail: HashMap<&'static str, &'static str>,
    /// why: fallback for self_text keys `match_spell_text` had to drop
    /// as ambiguous -- same text, same is_wearsoff split, coarser answer.
    self_landing_polarity: HashMap<&'static str, EffectPolarity>,
    self_wearsoff_polarity: HashMap<&'static str, EffectPolarity>,
    /// why: every spell behind a self text, shared or not -- the Group
    /// Buff Tracker's ledger needs "one of these landed on / left you"
    /// even when the text is shared by rank siblings (Clarity's own
    /// landing text is shared by 6 real spells)
    self_landing_all: HashMap<&'static str, Vec<&'static str>>,
    self_wearsoff_all: HashMap<&'static str, Vec<&'static str>>,
    /// why: same fallback for other_tail keys
    other_landing_polarity: HashMap<&'static str, EffectPolarity>,
}

/// why: `None` for text with no name to strip at all (rare, confirmed
/// real: `msg_cast_on_other` == "N/A"). pub(crate) -- ingest.rs's own
/// attribute_effect reuses this directly (see its own doc) for spell
/// lines this dictionary had to drop as globally ambiguous (the same
/// text shared by several rank/typo variants of one real spell line,
/// e.g. Tashania's own family) but which a real nearby cast can still
/// resolve locally, the same way it already does for msg_cast_on_you.
pub(crate) fn other_tail_of(msg: &str) -> Option<&str> {
    for p in PLACEHOLDERS {
        let Some(rest) = msg.strip_prefix(p) else {
            continue;
        };
        // why: real scrape quirk, 256 confirmed entries -- "Someone 's
        // X" (a stray space before the possessive marker), not
        // "Someone's X". Consuming any leading space(s) before checking
        // for "'s " unifies both spellings onto the same tail, matching
        // what the runtime "'s "-split path produces.
        let trimmed = rest.trim_start_matches(' ');
        if let Some(tail) = trimmed.strip_prefix("'s ") {
            return Some(tail);
        }
        // why: still requires a real space boundary after the
        // placeholder -- guards against `rest` being a coincidental
        // suffix of some other word starting with the same letters
        if rest.starts_with(' ') {
            return Some(trimmed);
        }
    }
    // why: real shape, confirmed -- "Banishing Poison"'s own
    // msg_cast_on_other is literally "'s blessings wither!", no leading
    // placeholder word at all
    if let Some(tail) = msg.strip_prefix("'s ") {
        return Some(tail);
    }
    // why: real shape, confirmed -- "Asp Venom"'s own msg_cast_on_other
    // is "coats their blades in asp venom!", already bare (log line is
    // "<Name> coats their blades..."); a lowercase first letter is what
    // distinguishes this from a message this function can't place
    if msg.starts_with(|c: char| c.is_lowercase()) {
        return Some(msg);
    }
    None
}

fn insert_unique<'a, V>(
    map: &mut HashMap<&'a str, V>,
    ambiguous: &mut HashSet<&'a str>,
    key: &'a str,
    value: V,
) {
    if ambiguous.contains(key) {
        return;
    }
    if map.contains_key(key) {
        map.remove(key);
        ambiguous.insert(key);
    } else {
        map.insert(key, value);
    }
}

fn build_dict() -> Dict {
    let mut self_text: HashMap<&str, (&str, bool)> = HashMap::new();
    let mut self_ambiguous: HashSet<&str> = HashSet::new();
    let mut other_tail: HashMap<&str, &str> = HashMap::new();
    let mut other_ambiguous: HashSet<&str> = HashSet::new();
    let mut self_landing_agg: HashMap<&str, PolarityAgg> = HashMap::new();
    let mut self_wearsoff_agg: HashMap<&str, PolarityAgg> = HashMap::new();
    let mut self_landing_all: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut self_wearsoff_all: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut other_landing_agg: HashMap<&str, PolarityAgg> = HashMap::new();

    for s in spelldata::spells() {
        let name = s.name.as_str();
        let polarity = s.spell_type.as_deref().and_then(spell_type_polarity);
        for (msg, is_wearsoff) in [
            (s.msg_cast_on_you.as_deref(), false),
            (s.msg_wears_off.as_deref(), true),
        ] {
            let Some(msg) = msg else { continue };
            if msg.is_empty() || msg == "N/A" {
                continue;
            }
            insert_unique(
                &mut self_text,
                &mut self_ambiguous,
                msg,
                (name, is_wearsoff),
            );
            let all = if is_wearsoff {
                &mut self_wearsoff_all
            } else {
                &mut self_landing_all
            };
            all.entry(msg).or_default().push(name);
            if let Some(p) = polarity {
                let agg = if is_wearsoff {
                    &mut self_wearsoff_agg
                } else {
                    &mut self_landing_agg
                };
                let entry = agg.entry(msg).or_insert(PolarityAgg::Unknown);
                *entry = std::mem::replace(entry, PolarityAgg::Unknown).add(p);
            }
        }
        if let Some(msg) = s.msg_cast_on_other.as_deref() {
            if msg.is_empty() || msg == "N/A" {
                continue;
            }
            if let Some(tail) = other_tail_of(msg) {
                insert_unique(&mut other_tail, &mut other_ambiguous, tail, name);
                if let Some(p) = polarity {
                    let entry = other_landing_agg
                        .entry(tail)
                        .or_insert(PolarityAgg::Unknown);
                    *entry = std::mem::replace(entry, PolarityAgg::Unknown).add(p);
                }
            }
        }
    }
    Dict {
        self_text,
        other_tail,
        self_landing_polarity: finalize_polarity(self_landing_agg),
        self_wearsoff_polarity: finalize_polarity(self_wearsoff_agg),
        self_landing_all,
        self_wearsoff_all,
        other_landing_polarity: finalize_polarity(other_landing_agg),
    }
}

fn dict() -> &'static Dict {
    static DICT: OnceLock<Dict> = OnceLock::new();
    DICT.get_or_init(build_dict)
}

/// why: mirrors ingest.rs's own third_person_flavor/verb_conjugated_flavor
/// technique (try every space / "'s " split as a candidate name boundary)
/// -- multi-word names (`Warlord Skarlon`) need every split tried, not
/// just the first.
pub fn match_spell_text(text: &str) -> Option<SpellTextMatch> {
    let d = dict();
    if let Some(&(spell, is_wearsoff)) = d.self_text.get(text) {
        return Some(SpellTextMatch {
            spell,
            target: "You".to_string(),
            is_wearsoff,
        });
    }
    for (idx, _) in text.match_indices("'s ") {
        if idx == 0 {
            continue;
        }
        let tail = &text[idx + 3..];
        if let Some(&spell) = d.other_tail.get(tail) {
            return Some(SpellTextMatch {
                spell,
                target: text[..idx].to_string(),
                is_wearsoff: false,
            });
        }
    }
    for (i, b) in text.bytes().enumerate() {
        if b != b' ' || i == 0 {
            continue;
        }
        let tail = &text[i + 1..];
        if let Some(&spell) = d.other_tail.get(tail) {
            return Some(SpellTextMatch {
                spell,
                target: text[..i].to_string(),
                is_wearsoff: false,
            });
        }
    }
    None
}

/// why: every spell whose msg_cast_on_you is exactly this text, shared
/// or not -- see Dict::self_landing_all
pub fn landing_candidates(text: &str) -> &'static [&'static str] {
    dict()
        .self_landing_all
        .get(text)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

/// why: same for msg_wears_off
pub fn wearsoff_candidates(text: &str) -> &'static [&'static str] {
    dict()
        .self_wearsoff_all
        .get(text)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

/// why: fallback for text `match_spell_text` had to drop as ambiguous --
/// call this only after `match_spell_text` returns `None`, same
/// self-text / possessive / space-split traversal, coarser answer
/// (polarity, not a spell name) when every real candidate agrees on one.
pub fn match_effect_polarity(text: &str) -> Option<EffectPolarityMatch> {
    let d = dict();
    if let Some(&polarity) = d.self_landing_polarity.get(text) {
        return Some(EffectPolarityMatch {
            polarity,
            target: "You".to_string(),
            is_wearsoff: false,
        });
    }
    if let Some(&polarity) = d.self_wearsoff_polarity.get(text) {
        return Some(EffectPolarityMatch {
            polarity,
            target: "You".to_string(),
            is_wearsoff: true,
        });
    }
    for (idx, _) in text.match_indices("'s ") {
        if idx == 0 {
            continue;
        }
        let tail = &text[idx + 3..];
        if let Some(&polarity) = d.other_landing_polarity.get(tail) {
            return Some(EffectPolarityMatch {
                polarity,
                target: text[..idx].to_string(),
                is_wearsoff: false,
            });
        }
    }
    for (i, b) in text.bytes().enumerate() {
        if b != b' ' || i == 0 {
            continue;
        }
        let tail = &text[i + 1..];
        if let Some(&polarity) = d.other_landing_polarity.get(tail) {
            return Some(EffectPolarityMatch {
                polarity,
                target: text[..i].to_string(),
                is_wearsoff: false,
            });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// why: real, confirmed-unambiguous packs/spells.json entry
    #[test]
    fn an_on_you_landing_message_resolves_to_its_spell() {
        let m = match_spell_text("You experience a quickening.").expect("known landing text");
        assert_eq!(m.spell, "Aanya's Quickening");
        assert_eq!(m.target, "You");
        assert!(!m.is_wearsoff);
    }

    /// why: real, confirmed-unambiguous packs/spells.json entry
    #[test]
    fn a_wears_off_message_resolves_and_is_flagged() {
        let m = match_spell_text("Your abducted strength fades.").expect("known wears-off text");
        assert_eq!(m.spell, "Ab of Strength Recourse");
        assert_eq!(m.target, "You");
        assert!(m.is_wearsoff);
    }

    /// why: real, confirmed-unambiguous packs/spells.json entry --
    /// verb-conjugated third-person shape ("Someone is X" -> "<Name> is X")
    #[test]
    fn a_verb_conjugated_third_person_message_strips_the_real_name() {
        let m = match_spell_text("Zimm is quickened by the Blessing of Reverence.")
            .expect("known third-person landing text");
        assert_eq!(m.spell, "Blessing of Piety");
        assert_eq!(m.target, "Zimm");
        assert!(!m.is_wearsoff);
    }

    /// why: real, confirmed-unambiguous packs/spells.json entry --
    /// possessive third-person shape ("Someone's X" -> "<Name>'s X")
    #[test]
    fn a_possessive_third_person_message_strips_the_real_name() {
        let m = match_spell_text("Kaeus's muscles pulse with abducted strength.")
            .expect("known possessive landing text");
        assert_eq!(m.spell, "Ab of Strength Recourse");
        assert_eq!(m.target, "Kaeus");
        assert!(!m.is_wearsoff);
    }

    /// why: real, confirmed-unambiguous packs/spells.json entry --
    /// no placeholder in the source text at all, tail used verbatim
    #[test]
    fn a_placeholder_less_third_person_message_still_resolves() {
        let m = match_spell_text("Bigneum coats their blades in asp venom!")
            .expect("known placeholder-less landing text");
        assert_eq!(m.spell, "Asp Venom");
        assert_eq!(m.target, "Bigneum");
    }

    /// why: real catalog case, confirmed shared by 40+ genuinely
    /// different spells (every gate/port/translocate) -- must not guess
    #[test]
    fn text_shared_by_many_spells_matches_nothing() {
        assert!(match_spell_text("Someone fades away.").is_none());
        assert!(match_spell_text("A very unpleasant hand fades away.").is_none());
    }

    /// why: real catalog case, confirmed shared by 9 spells with the
    /// same "no reuse" utility shape (Alacrity/Celerity/Flurry/etc.)
    #[test]
    fn ambiguous_wears_off_text_is_dropped_not_guessed() {
        assert!(match_spell_text("Your speed returns to normal.").is_none());
    }

    #[test]
    fn unrecognized_text_matches_nothing() {
        assert!(match_spell_text("You hit a gnoll for 5 points of damage.").is_none());
    }

    /// why: real bug caught against the real log -- "Vampiric Embrace"'s
    /// own msg_cast_on_other is "Someone 's hands begin to glow." (a
    /// stray space before the possessive marker, 256 spells share this
    /// scrape quirk), which built a "'s hands begin to glow." tail while
    /// the runtime "'s "-split path looks up "hands begin to glow." --
    /// two different keys for the same spell, so it never matched a
    /// single real line until other_tail_of normalized both onto one tail.
    #[test]
    fn a_placeholder_with_a_stray_space_before_the_possessive_still_resolves() {
        let m = match_spell_text("Dippinsauce's hands begin to glow.")
            .expect("known landing text despite the scrape's stray space");
        assert_eq!(m.spell, "Vampiric Embrace");
        assert_eq!(m.target, "Dippinsauce");
    }

    /// why: real, confirmed live -- "Your feet come free." is shared by
    /// 9 real spells (Root/Fetter/Immobilize/Paralyzing Poison/etc, real
    /// distinct lines, not rank siblings), so match_spell_text drops it,
    /// but every one of those 9 is spell_type Detrimental -- a real,
    /// confident debuff even with no confident name.
    #[test]
    fn ambiguous_wearsoff_text_still_resolves_a_polarity_every_candidate_agrees_on() {
        assert!(match_spell_text("Your feet come free.").is_none());
        let m = match_effect_polarity("Your feet come free.").expect("known polarity fallback");
        assert_eq!(m.polarity, EffectPolarity::Debuff);
        assert_eq!(m.target, "You");
        assert!(m.is_wearsoff);
    }

    /// why: real, confirmed live against the reference log -- "Lenekab
    /// is surrounded by a brief lupine aura." shares its tail with 4
    /// real SoW-family spells (Pack Spirit/Spirit of Bih`Li/Spirit of
    /// Scale/Spirit of Wolf), all Movement Buff/Buff/Beneficial -- a
    /// confident buff, third-person, name stripped same as match_spell_text.
    #[test]
    fn ambiguous_third_person_landing_text_resolves_a_polarity_with_the_name_stripped() {
        assert!(match_spell_text("Lenekab is surrounded by a brief lupine aura.").is_none());
        let m = match_effect_polarity("Lenekab is surrounded by a brief lupine aura.")
            .expect("known polarity fallback");
        assert_eq!(m.polarity, EffectPolarity::Buff);
        assert_eq!(m.target, "Lenekab");
        assert!(!m.is_wearsoff);
    }

    /// why: real, confirmed live -- "You feel much better." is shared by
    /// 8 real Heal-line spells (Greater/Regular Healing, Word of
    /// Healing/Health, Invigorate, Knight's Blessing, Nature's Touch,
    /// Superior Healing), every one Heal/Beneficial -- self, non-wearsoff.
    #[test]
    fn ambiguous_self_landing_text_resolves_a_polarity_too() {
        let m = match_effect_polarity("You feel much better.").expect("known polarity fallback");
        assert_eq!(m.polarity, EffectPolarity::Buff);
        assert_eq!(m.target, "You");
        assert!(!m.is_wearsoff);
    }

    /// why: real catalog case -- "Kaeus sinks into the ground." is
    /// shared by Earth Elemental Attack/EarthElementalAttack (Root, i.e.
    /// detrimental) and Egress (a beneficial escape teleport) -- a real
    /// disagreement, not just an unclassified spell_type, so this must
    /// stay dropped rather than pick a side.
    #[test]
    fn a_genuine_polarity_disagreement_is_dropped_not_guessed() {
        assert!(match_effect_polarity("Kaeus sinks into the ground.").is_none());
    }

    #[test]
    fn unrecognized_text_has_no_polarity_either() {
        assert!(match_effect_polarity("You hit a gnoll for 5 points of damage.").is_none());
    }
}

/// why: an AoE lands ONE line per target, and the target's name is the
/// only thing that varies -- every `msg_cast_on_other` in the pack reads
/// "Someone <predicate>.", so a log line is "<name> <predicate>.". 798
/// distinct predicates, so this is the spell data answering rather than a
/// curated list. Returns the name and the predicate; several of one name
/// on one predicate in one instant is a census of that pull.
///
/// A name can be any number of words, so the split is found by trying each
/// space and asking whether the tail is a predicate the catalog knows.
pub fn cast_on_other(text: &str) -> Option<(&str, &'static str)> {
    static IDX: OnceLock<std::collections::HashSet<&'static str>> = OnceLock::new();
    // why: only spells that AREA target. A single-target spell lands once,
    // so a repeat of its message in one instant means two casters hit one
    // mob, not two mobs -- "Someone winces." is shared by Chords of
    // Dissonance (PB AE) and Cannibalize (Self), and counting the second
    // would inflate rather than floor. 156 messages survive the filter.
    let idx = IDX.get_or_init(|| {
        const AREA: &[&str] = &["AE", "Free Target AE", "PB AE", "PBAOE", "Targeted AE"];
        crate::spelldata::spells()
            .iter()
            .filter(|s| s.target_type.as_deref().is_some_and(|t| AREA.contains(&t)))
            .filter_map(|s| s.msg_cast_on_other.as_deref())
            .filter_map(|m| m.strip_prefix("Someone "))
            .collect()
    });
    let mut start = 0;
    while let Some(off) = text[start..].find(' ') {
        let cut = start + off + 1;
        if let Some(hit) = idx.get(&text[cut..]) {
            // why: a bare predicate with no name in front is not a landing
            return (cut > 1).then_some((&text[..cut - 1], *hit));
        }
        start = cut;
    }
    None
}

#[cfg(test)]
mod cast_on_other_tests {
    use super::*;

    /// why: an AoE lands one line per target, so the split between the
    /// target's NAME and the spell's own message is what makes a census
    /// possible. The name can be any number of words, so the split is
    /// found by asking the catalog.
    #[test]
    fn a_landing_splits_into_target_and_message() {
        let (who, effect) =
            cast_on_other("a gnoll is stunned by scintillating colors.").expect("a known landing");
        assert_eq!(who, "a gnoll");
        assert_eq!(effect, "is stunned by scintillating colors.");

        // why: multi-word names are the normal case
        let (who, _) = cast_on_other("An Amygdalan knight winces.").expect("PB AE bard song");
        assert_eq!(who, "An Amygdalan knight");
    }

    /// why: only spells that AREA target fan out. A single-target message
    /// repeating in one instant means two casters hit ONE mob, which would
    /// inflate the count rather than floor it.
    #[test]
    fn a_single_target_message_is_not_a_census() {
        // why: "Someone's body convulses." is Single-target; if this ever
        // starts matching, the filter has been loosened
        for text in [
            "a gnoll has been mesmerized.",
            "a gnoll staggers under the assault.",
        ] {
            if let Some((_, e)) = cast_on_other(text) {
                let area = spelldata::spells().iter().any(|s| {
                    s.msg_cast_on_other.as_deref() == Some(&format!("Someone {e}"))
                        && matches!(
                            s.target_type.as_deref(),
                            Some("AE" | "Free Target AE" | "PB AE" | "PBAOE" | "Targeted AE")
                        )
                });
                assert!(area, "{text:?} matched a non-area spell");
            }
        }
    }

    /// why: a line with no name in front is not a landing
    #[test]
    fn a_bare_message_is_not_a_landing() {
        assert!(cast_on_other("winces.").is_none());
        assert!(cast_on_other("").is_none());
    }
}

/// why: a resisted target prints a resist line INSTEAD of a landing, so
/// the cast's real target count is landings + resists. Returns the spell's
/// own landing message so a resist lands in the SAME fan-out bucket the
/// landings do; falls back to the spell name for area spells that print
/// nothing on the target.
pub fn area_effect_of(name: &str) -> Option<&'static str> {
    static IDX: OnceLock<std::collections::HashMap<String, &'static str>> = OnceLock::new();
    IDX.get_or_init(|| {
        crate::spelldata::spells()
            .iter()
            .filter(|s| {
                matches!(
                    s.target_type.as_deref(),
                    Some("AE" | "Free Target AE" | "PB AE" | "PBAOE" | "Targeted AE")
                )
            })
            .map(|s| {
                let effect = s
                    .msg_cast_on_other
                    .as_deref()
                    .and_then(|m| m.strip_prefix("Someone "))
                    .unwrap_or(s.name.as_str());
                (s.name.to_lowercase(), effect)
            })
            .collect()
    })
    .get(&name.to_lowercase())
    .copied()
}

#[cfg(test)]
mod area_effect_tests {
    use super::*;

    /// why: the resist has to land in the same bucket the landings do, or
    /// the two halves of one cast are counted apart
    #[test]
    fn an_area_spell_reports_the_message_its_landings_use() {
        let effect = area_effect_of("Color Flux").expect("Color Flux is PB AE");
        assert_eq!(
            cast_on_other(&format!("a gnoll {effect}")).map(|(_, e)| e),
            Some(effect),
            "a landing of the same spell must key the same"
        );
    }

    /// why: two casters resisting on ONE mob in one instant is not two mobs
    #[test]
    fn a_single_target_spell_has_no_area_effect() {
        assert_eq!(area_effect_of("Shock of Ice"), None);
    }
}
