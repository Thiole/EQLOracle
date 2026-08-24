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
}

/// why: `None` for text with no name to strip at all (rare, confirmed
/// real: `msg_cast_on_other` == "N/A")
fn other_tail_of(msg: &str) -> Option<&str> {
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

    for s in spelldata::spells() {
        let name = s.name.as_str();
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
        }
        if let Some(msg) = s.msg_cast_on_other.as_deref() {
            if msg.is_empty() || msg == "N/A" {
                continue;
            }
            if let Some(tail) = other_tail_of(msg) {
                insert_unique(&mut other_tail, &mut other_ambiguous, tail, name);
            }
        }
    }
    Dict {
        self_text,
        other_tail,
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
}
