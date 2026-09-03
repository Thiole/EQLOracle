//! why: derived spell mechanics (duration, damage/heal/buff/debuff/
//! control components, category tags), computed from `spelldata::Spell`'s
//! raw fields, not a separate scrape -- pure interpretation, kept apart
//! from the raw data so a bad guess never corrupts the catalog itself.
//!
//! Two sources, in priority order:
//! 1. **`slots`** -- structured effect rows, reliable when present, but
//!    genuinely absent for some spells (Fire Bolt, an obvious nuke, has none).
//! 2. **`description`** -- free prose fallback only when `slots` has
//!    nothing damage/heal-shaped; 191 real Detrimental spells with empty
//!    slots still say "damage" in their own description.
//!
//! Neither source is exhaustive -- an unmatched spell gets no component
//! at all, an honest gap. Every parsed field keeps its raw source text.

use crate::spelldata::Spell;
use regex::Regex;
use serde::Serialize;
use std::sync::OnceLock;

/// why: one EQ tick in seconds, genre-standard, not confirmed for this fork
const TICK_SECS: f64 = 6.0;

#[derive(Debug, Clone, Serialize)]
pub struct SpellDuration {
    /// why: None for Permanent or unparseable; equal to max_secs unless leveled range
    pub min_secs: Option<f64>,
    pub max_secs: Option<f64>,
    pub is_instant: bool,
    pub is_permanent: bool,
    /// why: catalog's own string, always kept -- checkable against what it actually said
    pub raw: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EffectComponent {
    /// why: wiki's own stat name, not normalized against a fixed enum -- vocabulary too large
    pub stat: String,
    pub direction: String, // "increase" | "decrease"
    /// why: whether the amount is a per-tick DoT/HoT rate vs a flat/instant one
    pub per_tick: bool,
    pub unit: String, // "flat" | "percent"
    pub min_amount: Option<f64>,
    pub max_amount: Option<f64>,
    pub raw: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SpellEffects {
    pub duration: SpellDuration,
    /// why: from `slots`; prose doesn't reliably carry which stat changed,
    /// only the number, which description_damage/heal carry instead
    pub components: Vec<EffectComponent>,
    /// why: found control labels, a spell can have none, one, or several
    pub control: Vec<String>,
    /// why: set only when slots had no HP entry and description matched
    /// a known phrasing; is_over_time mirrors per_tick's meaning
    pub description_damage: Option<DescriptionAmount>,
    pub description_heal: Option<DescriptionAmount>,
    /// why: best-effort category pills, several can apply at once (not mutually exclusive)
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DescriptionAmount {
    pub min_amount: f64,
    pub max_amount: f64,
    pub is_over_time: bool,
    /// why: repetitions the prose itself states, never guessed from target_type
    pub repetitions: Option<u32>,
}

// ---------------------------------------------------------------- duration

fn at_level_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)\s*@l\d+").unwrap())
}

fn hms_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^(\d+):(\d+)(?::(\d+))?").unwrap())
}

fn unit_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)(\d+(?:\.\d+)?)\s*(hours?|hrs?|ticks?|min(?:ute)?s?|sec(?:ond)?s?|s)\b")
            .unwrap()
    })
}

fn range_ticks_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)(\d+)\s*-\s*(\d+)\s*ticks?").unwrap())
}

fn unit_secs(unit: &str) -> f64 {
    let u = unit.to_ascii_lowercase();
    if u.starts_with("hour") || u.starts_with("hr") {
        3600.0
    } else if u.starts_with("tick") {
        TICK_SECS
    } else if u.starts_with("min") {
        60.0
    } else {
        1.0 // sec(s)/second(s)/s
    }
}

/// why: sums every `<number><unit>` token; a same-unit range ("N-N
/// ticks") handled first, else the plain scan would add both as separate units
fn parse_duration_fragment(s: &str) -> Option<f64> {
    if let Some(caps) = range_ticks_re().captures(s) {
        let hi: f64 = caps[2].parse().ok()?;
        return Some(hi * TICK_SECS);
    }
    let mut total = 0.0;
    let mut found = false;
    for caps in unit_re().captures_iter(s) {
        let n: f64 = caps[1].parse().ok()?;
        total += n * unit_secs(&caps[2]);
        found = true;
    }
    found.then_some(total)
}

/// `raw` is the catalog's own `duration` string, verbatim.
pub fn parse_duration(raw: Option<&str>) -> SpellDuration {
    let Some(raw) = raw else {
        return SpellDuration {
            min_secs: None,
            max_secs: None,
            is_instant: false,
            is_permanent: false,
            raw: None,
        };
    };
    let trimmed = raw.trim();
    let empty = || SpellDuration {
        min_secs: None,
        max_secs: None,
        is_instant: false,
        is_permanent: false,
        raw: Some(raw.to_string()),
    };
    // why: starts_with/contains not exact match -- real variants like
    // "Instant (until zoning/recast)" would miss an exact check
    if trimmed.to_ascii_lowercase().starts_with("instant") {
        return SpellDuration {
            min_secs: Some(0.0),
            max_secs: Some(0.0),
            is_instant: true,
            is_permanent: false,
            raw: Some(raw.to_string()),
        };
    }
    if trimmed.eq_ignore_ascii_case("permanent") || trimmed.eq_ignore_ascii_case("unlimited") {
        return SpellDuration {
            min_secs: None,
            max_secs: None,
            is_instant: false,
            is_permanent: true,
            raw: Some(raw.to_string()),
        };
    }
    // why: leveled range shape, "@L" proves it so this never misfires on
    // an ordinary combined-unit duration
    if trimmed.contains("@L") {
        if let Some(idx) = trimmed.find(" to ") {
            let (left, right) = trimmed.split_at(idx);
            let right = &right[" to ".len()..];
            let min = parse_duration_fragment(&at_level_re().replace_all(left, ""));
            let max = parse_duration_fragment(&at_level_re().replace_all(right, ""));
            return SpellDuration {
                min_secs: min,
                max_secs: max,
                is_instant: false,
                is_permanent: false,
                raw: Some(raw.to_string()),
            };
        }
        // why: single "@L" no range -- strip and parse the one value
        let stripped = at_level_re().replace_all(trimmed, "");
        let secs = parse_duration_fragment(&stripped);
        return SpellDuration {
            min_secs: secs,
            max_secs: secs,
            is_instant: false,
            is_permanent: false,
            raw: Some(raw.to_string()),
        };
    }
    // why: first H:M:S is the primary value; a parenthetical alternate
    // duration stays in `raw`, not separately parsed
    if let Some(caps) = hms_re().captures(trimmed) {
        let a: f64 = caps[1].parse().ok().unwrap_or(0.0);
        let b: f64 = caps[2].parse().ok().unwrap_or(0.0);
        let secs = match caps.get(3) {
            Some(c) => a * 3600.0 + b * 60.0 + c.as_str().parse::<f64>().unwrap_or(0.0), // H:M:S
            None => a * 60.0 + b,                                                        // M:S
        };
        return SpellDuration {
            min_secs: Some(secs),
            max_secs: Some(secs),
            is_instant: false,
            is_permanent: false,
            raw: Some(raw.to_string()),
        };
    }
    let secs = parse_duration_fragment(trimmed);
    if secs.is_none() {
        return empty();
    }
    SpellDuration {
        min_secs: secs,
        max_secs: secs,
        is_instant: false,
        is_permanent: false,
        raw: Some(raw.to_string()),
    }
}

// ---------------------------------------------------------------- slots (structured components)

fn component_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)^(?P<dir>increase|decrease)\s+(?P<stat>.+?)\s+(?:when cast\s+)?by\s*(?P<amt1>\d+(?:\.\d+)?)\s*(?P<pct1>%)?(?:\s*\(l\d+\))?(?:\s*(?:to|-)\s*(?P<amt2>\d+(?:\.\d+)?)\s*(?P<pct2>%)?(?:\s*\(l\d+\))?)?",
        )
        .unwrap()
    })
}

fn parse_component(raw: &str) -> Option<EffectComponent> {
    let caps = component_re().captures(raw)?;
    let direction = caps["dir"].to_ascii_lowercase();
    let stat = caps["stat"].trim().to_string();
    let min_amount: f64 = caps["amt1"].parse().ok()?;
    let max_amount: f64 = caps
        .name("amt2")
        .and_then(|m| m.as_str().parse().ok())
        .unwrap_or(min_amount);
    let unit = if caps.name("pct1").is_some() || caps.name("pct2").is_some() {
        "percent"
    } else {
        "flat"
    };
    let per_tick = raw.to_ascii_lowercase().contains("per tick");
    Some(EffectComponent {
        stat,
        direction,
        per_tick,
        unit: unit.to_string(),
        min_amount: Some(min_amount),
        max_amount: Some(max_amount),
        raw: raw.to_string(),
    })
}

fn mesmerize_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)^mesmerize\b").unwrap())
}
fn charm_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)^charm\b").unwrap())
}
fn fear_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)^fear\(").unwrap())
}
fn stun_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)^(stun\b|spinstun)").unwrap())
}

/// why: None for an unrecognized row -- absent from both lists rather than guessed at
enum SlotEffect {
    Component(EffectComponent),
    Control(&'static str),
}

fn parse_slot(raw: &str) -> Option<SlotEffect> {
    if mesmerize_re().is_match(raw) {
        return Some(SlotEffect::Control("Mesmerize"));
    }
    if charm_re().is_match(raw) {
        return Some(SlotEffect::Control("Charm"));
    }
    if fear_re().is_match(raw) {
        return Some(SlotEffect::Control("Fear"));
    }
    if stun_re().is_match(raw) {
        return Some(SlotEffect::Control("Stun"));
    }
    parse_component(raw).map(SlotEffect::Component)
}

// ---------------------------------------------------------------- description fallback

// why: ranges/over-time tried before plain "N damage" catch-alls, else a
// looser pattern would short-circuit and lose the DoT/duration context
fn desc_between_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)between (\d+) and (\d+) (?:hit points|damage)").unwrap())
}
fn desc_waves_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)waves? of (\d+) damage").unwrap())
}
fn desc_over_time_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // why: lazy match covers whatever noun sits before "every", not just "hit points"
    RE.get_or_init(|| {
        Regex::new(r"(?i)(\d+)[a-z\s]*?\bevery\b\s+[a-z0-9 ]+?\s*(?:seconds?|secs?)").unwrap()
    })
}
fn desc_flat_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)(?:causing|doing|inflicts?|deals?) (\d+)(?: points?)?(?: of [a-z]+)? damage",
        )
        .unwrap()
    })
}
fn desc_points_of_damage_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)(\d+) points? of (?:[a-z]+ )?damage").unwrap())
}
fn desc_heal_flat_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)heal(?:s|ing)?(?: for)? (\d+)(?: hit points)?").unwrap())
}
fn desc_repetitions_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)maximum of (\d+) targets|up to (\d+) (?:enem(?:y|ies)|targets?|creatures?)",
        )
        .unwrap()
    })
}

fn repetitions_in(desc: &str) -> Option<u32> {
    let caps = desc_repetitions_re().captures(desc)?;
    caps.get(1).or_else(|| caps.get(2))?.as_str().parse().ok()
}

fn description_damage(desc: &str) -> Option<DescriptionAmount> {
    if let Some(caps) = desc_between_re().captures(desc) {
        let a: f64 = caps[1].parse().ok()?;
        let b: f64 = caps[2].parse().ok()?;
        return Some(DescriptionAmount {
            min_amount: a.min(b),
            max_amount: a.max(b),
            is_over_time: false,
            repetitions: repetitions_in(desc),
        });
    }
    if let Some(caps) = desc_waves_re().captures(desc) {
        let n: f64 = caps[1].parse().ok()?;
        return Some(DescriptionAmount {
            min_amount: n,
            max_amount: n,
            is_over_time: false,
            repetitions: repetitions_in(desc),
        });
    }
    if let Some(caps) = desc_over_time_re().captures(desc) {
        let n: f64 = caps[1].parse().ok()?;
        return Some(DescriptionAmount {
            min_amount: n,
            max_amount: n,
            is_over_time: true,
            repetitions: None,
        });
    }
    for pattern_fn in [desc_flat_re, desc_points_of_damage_re] {
        if let Some(caps) = pattern_fn().captures(desc) {
            let n: f64 = caps[1].parse().ok()?;
            return Some(DescriptionAmount {
                min_amount: n,
                max_amount: n,
                is_over_time: false,
                repetitions: repetitions_in(desc),
            });
        }
    }
    None
}

fn description_heal(desc: &str) -> Option<DescriptionAmount> {
    if let Some(caps) = desc_between_re().captures(desc) {
        let a: f64 = caps[1].parse().ok()?;
        let b: f64 = caps[2].parse().ok()?;
        return Some(DescriptionAmount {
            min_amount: a.min(b),
            max_amount: a.max(b),
            is_over_time: false,
            repetitions: None,
        });
    }
    if let Some(caps) = desc_over_time_re().captures(desc) {
        let n: f64 = caps[1].parse().ok()?;
        return Some(DescriptionAmount {
            min_amount: n,
            max_amount: n,
            is_over_time: true,
            repetitions: None,
        });
    }
    if let Some(caps) = desc_heal_flat_re().captures(desc) {
        let n: f64 = caps[1].parse().ok()?;
        return Some(DescriptionAmount {
            min_amount: n,
            max_amount: n,
            is_over_time: false,
            repetitions: None,
        });
    }
    None
}

// ---------------------------------------------------------------- category tags

const HP_STATS: &[&str] = &["hitpoints", "hit points", "hp", "current hit points"];

/// why: combines slots-derived components/control with spell_type/
/// target_type and the description fallback; not mutually exclusive
fn categorize(
    components: &[EffectComponent],
    control: &[String],
    spell: &Spell,
    desc_damage: &Option<DescriptionAmount>,
    desc_heal: &Option<DescriptionAmount>,
) -> Vec<String> {
    let mut tags = Vec::new();
    let is_ae = spell
        .target_type
        .as_deref()
        .is_some_and(|t| t.to_ascii_lowercase().contains("ae"));
    let ae_prefix = if is_ae { "AE " } else { "" };

    let hp_component = |dir: &str, per_tick: bool| {
        components.iter().any(|c| {
            HP_STATS.contains(&c.stat.to_ascii_lowercase().as_str())
                && c.direction == dir
                && c.per_tick == per_tick
        })
    };

    if hp_component("decrease", false)
        || (components.is_empty() && desc_damage.as_ref().is_some_and(|d| !d.is_over_time))
    {
        tags.push(format!("{ae_prefix}Damage"));
    }
    if hp_component("decrease", true)
        || (components.is_empty() && desc_damage.as_ref().is_some_and(|d| d.is_over_time))
    {
        tags.push(format!("{ae_prefix}Damage over Time"));
    }
    if hp_component("increase", false)
        || (components.is_empty() && desc_heal.as_ref().is_some_and(|d| !d.is_over_time))
    {
        tags.push("Heal".to_string());
    }
    if hp_component("increase", true)
        || (components.is_empty() && desc_heal.as_ref().is_some_and(|d| d.is_over_time))
    {
        tags.push("Heal over Time".to_string());
    }
    if control.iter().any(|c| c == "Mesmerize") {
        tags.push("Mez".to_string());
    }
    if control.iter().any(|c| c == "Charm") {
        tags.push("Charm".to_string());
    }
    if control.iter().any(|c| c == "Fear") {
        tags.push("Fear".to_string());
    }
    if control.iter().any(|c| c == "Stun") {
        tags.push("Stun".to_string());
    }
    if components
        .iter()
        .any(|c| c.stat.eq_ignore_ascii_case("movement speed") && c.direction == "decrease")
    {
        tags.push("Snare".to_string());
    }

    // why: only when nothing more specific applies -- avoids a redundant bare "Debuff"
    if tags.is_empty() {
        match spell.spell_type.as_deref() {
            Some(t) if t.eq_ignore_ascii_case("beneficial") => tags.push("Buff".to_string()),
            Some(t) if t.eq_ignore_ascii_case("detrimental") => tags.push("Debuff".to_string()),
            _ => tags.push("Utility".to_string()),
        }
    }
    tags
}

/// why: everything derived for one catalog spell
pub fn effects_for(spell: &Spell) -> SpellEffects {
    let duration = parse_duration(spell.duration.as_deref());

    let mut components = Vec::new();
    let mut control = Vec::new();
    for slot in &spell.slots {
        match parse_slot(&slot.effect) {
            Some(SlotEffect::Component(c)) => components.push(c),
            Some(SlotEffect::Control(label)) => control.push(label.to_string()),
            None => {}
        }
    }

    let has_hp_component = components
        .iter()
        .any(|c| HP_STATS.contains(&c.stat.to_ascii_lowercase().as_str()));
    let desc = spell.description.as_deref().unwrap_or("");
    let description_damage = (!has_hp_component)
        .then(|| description_damage(desc))
        .flatten();
    let description_heal = (!has_hp_component && description_damage.is_none())
        .then(|| description_heal(desc))
        .flatten();

    let tags = categorize(
        &components,
        &control,
        spell,
        &description_damage,
        &description_heal,
    );

    SpellEffects {
        duration,
        components,
        control,
        description_damage,
        description_heal,
        tags,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SpellEffectsEntry {
    pub id: String,
    #[serde(flatten)]
    pub effects: SpellEffects,
}

static ALL_EFFECTS: OnceLock<Vec<SpellEffectsEntry>> = OnceLock::new();

/// why: computed once and cached -- pure function of the static catalog,
/// never stale; two consumers read this list keyed by id rather than re-deriving
pub fn all_effects() -> &'static [SpellEffectsEntry] {
    ALL_EFFECTS.get_or_init(|| {
        crate::spelldata::spells()
            .iter()
            .map(|s| SpellEffectsEntry {
                id: s.id.clone(),
                effects: effects_for(s),
            })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_handles_the_real_format_variety() {
        assert!(parse_duration(Some("Instant")).is_instant);
        assert!(parse_duration(Some("Permanent")).is_permanent);
        assert_eq!(parse_duration(Some("48 Sec")).max_secs, Some(48.0));
        assert_eq!(parse_duration(Some("4 ticks")).max_secs, Some(24.0));
        assert_eq!(
            parse_duration(Some("2 hours 0 minutes")).max_secs,
            Some(7200.0)
        );
        assert_eq!(parse_duration(Some("2 Min 30 Sec")).max_secs, Some(150.0));
        let hms = parse_duration(Some("2:24:00 (3:36:00)"));
        assert_eq!(hms.max_secs, Some(2.0 * 3600.0 + 24.0 * 60.0));
        let ranged = parse_duration(Some("3 minutes @L1 to 25 minutes @L9"));
        assert_eq!(ranged.min_secs, Some(180.0));
        assert_eq!(ranged.max_secs, Some(1500.0));
        let single_level = parse_duration(Some("7 minutes @L60"));
        assert_eq!(single_level.max_secs, Some(420.0));
        let tick_range = parse_duration(Some("36-40 ticks"));
        assert_eq!(tick_range.max_secs, Some(240.0));
        assert_eq!(parse_duration(None).raw, None);
    }

    #[test]
    fn suffocate_gets_an_instant_hit_and_a_dot_tick_and_two_debuffs() {
        // Real slots from the reference catalog (Suffocate).
        let raw = [
            ("Decrease HP when cast by 34 (L29) to 65 (L60)", false),
            ("Decrease STR by 15 (L29) to 20 (L38)", false),
            ("Decrease AGI by 15 (L29) to 20 (L38)", false),
            ("Decrease Hitpoints by 11 per tick", false),
        ];
        let components: Vec<EffectComponent> =
            raw.iter().filter_map(|(s, _)| parse_component(s)).collect();
        assert_eq!(components.len(), 4);
        assert_eq!(components[0].stat, "HP");
        assert_eq!(components[0].min_amount, Some(34.0));
        assert_eq!(components[0].max_amount, Some(65.0));
        assert!(!components[0].per_tick);
        assert_eq!(components[1].stat, "STR");
        assert_eq!(components[3].stat, "Hitpoints");
        assert!(components[3].per_tick);
        assert_eq!(components[3].min_amount, Some(11.0));
    }

    #[test]
    fn percent_and_no_space_variants_still_parse() {
        let c = parse_component("Decrease Attack Speed by 10%").unwrap();
        assert_eq!(c.unit, "percent");
        assert_eq!(c.min_amount, Some(10.0));
        let c2 = parse_component("Decrease STR by25").unwrap(); // real scrape typo, no space
        assert_eq!(c2.min_amount, Some(25.0));
        let c3 = parse_component("Decrease Movement Speed by 70-90%").unwrap(); // dash range, no @L
        assert_eq!(c3.min_amount, Some(70.0));
        assert_eq!(c3.max_amount, Some(90.0));
        assert_eq!(c3.unit, "percent");
    }

    #[test]
    fn control_effects_are_recognized() {
        assert!(matches!(
            parse_slot("Mesmerize (2/55)"),
            Some(SlotEffect::Control("Mesmerize"))
        ));
        assert!(matches!(
            parse_slot("Charm up to level 25"),
            Some(SlotEffect::Control("Charm"))
        ));
        assert!(matches!(
            parse_slot("Fear(1)"),
            Some(SlotEffect::Control("Fear"))
        ));
        assert!(matches!(
            parse_slot("Stun for 3.00s"),
            Some(SlotEffect::Control("Stun"))
        ));
        assert!(matches!(
            parse_slot("SpinStun"),
            Some(SlotEffect::Control("Stun"))
        ));
    }

    /// why: Fire Bolt has empty slots -- the real case the fallback exists for
    #[test]
    fn fire_bolt_gets_its_damage_number_from_description_not_slots() {
        let dmg =
            description_damage("Creates a bolt of fire that burns your target, causing 65 damage.")
                .unwrap();
        assert_eq!(dmg.min_amount, 65.0);
        assert_eq!(dmg.max_amount, 65.0);
        assert!(!dmg.is_over_time);
    }

    #[test]
    fn description_dot_and_range_and_ae_repetitions_parse() {
        let dot = description_damage(
            "Fills your target's veins with pain, causing 68 damage every 6 seconds for 36s.",
        )
        .unwrap();
        assert_eq!(dot.min_amount, 68.0);
        assert!(dot.is_over_time);

        let ranged = description_damage(
            "Summons a whirling wind to stun your target, also doing between 772 and 812 damage.",
        )
        .unwrap();
        assert_eq!(ranged.min_amount, 772.0);
        assert_eq!(ranged.max_amount, 812.0);

        let ae = description_damage("Calls down a hailstorm from the sky, causing three waves of 125 damage, up to a maximum of 4 targets.").unwrap();
        assert_eq!(ae.min_amount, 125.0);
        assert_eq!(ae.repetitions, Some(4));

        let heal = description_heal("Fills your target's body with a celestial echo, immediately healing between 310 and 480 hit points.").unwrap();
        assert_eq!(heal.min_amount, 310.0);
        assert_eq!(heal.max_amount, 480.0);
    }

    #[test]
    fn category_tags_combine_correctly_for_a_real_multi_effect_spell() {
        let spell = Spell {
            id: "Suffocate".to_string(),
            name: "Suffocate".to_string(),
            url: None,
            description: Some("Chokes the air from your target's lungs.".to_string()),
            classes: vec![],
            skill: None,
            mana: Some(60.0),
            range: None,
            casting_time: Some(3.0),
            fizzle_time: None,
            recast_time: None,
            duration: Some("48 Sec".to_string()),
            target_type: Some("Single".to_string()),
            spell_type: Some("Detrimental".to_string()),
            resist: None,
            msg_cast_on_you: None,
            msg_cast_on_other: None,
            msg_wears_off: None,
            slots: vec![
                crate::spelldata::SpellSlot {
                    slot: 1,
                    effect: "Decrease HP when cast by 34 (L29) to 65 (L60)".to_string(),
                },
                crate::spelldata::SpellSlot {
                    slot: 2,
                    effect: "Decrease STR by 15 (L29) to 20 (L38)".to_string(),
                },
                crate::spelldata::SpellSlot {
                    slot: 4,
                    effect: "Decrease Hitpoints by 11 per tick".to_string(),
                },
            ],
            items_with_effect: vec![],
            where_to_obtain: None,
            era: None,
            categories: vec![],
            icon: None,
        };
        let effects = effects_for(&spell);
        assert!(effects.tags.contains(&"Damage".to_string()));
        assert!(effects.tags.contains(&"Damage over Time".to_string()));
    }

    /// why: coverage check, catalog history -- used to assert 50+ slotless
    /// spells, a fresh re-scrape found the wiki's slots data got much more
    /// complete (now only 2 slotless, both Detrimental). Asserts today's
    /// small population by name so a future re-scrape gets noticed here.
    #[test]
    fn only_two_real_spells_are_still_slotless_and_neither_is_damage_shaped() {
        let slotless: Vec<&Spell> = crate::spelldata::spells()
            .iter()
            .filter(|s| s.slots.is_empty())
            .collect();
        let names: Vec<&str> = slotless.iter().map(|s| s.name.as_str()).collect();
        // why: the 2026-09-03 re-scrape added the two Improved Vampirism
        // foci from Category:Focus Effects -- their wiki pages carry no
        // slot data at all, so they are slotless too (and not damage-shaped)
        assert_eq!(
            names,
            vec![
                "Improved Vampirism II",
                "Improved Vampirism III",
                "Instill",
                "Resurrection Effects"
            ],
            "the real slotless-spell population changed -- see this test's own doc"
        );
        for s in &slotless {
            assert!(
                description_damage(s.description.as_deref().unwrap_or("")).is_none(),
                "{} isn't damage-shaped (a root/snare and a rez-debuff meta-description) -- description_damage matching it would be a false positive",
                s.name
            );
        }
    }

    /// why: same coverage discipline for parse_duration against the whole real catalog
    #[test]
    fn duration_parses_the_large_majority_of_real_catalog_strings() {
        let with_duration: Vec<&Spell> = crate::spelldata::spells()
            .iter()
            .filter(|s| s.duration.is_some())
            .collect();
        assert!(
            with_duration.len() > 1000,
            "sanity: almost every real spell should carry a duration string"
        );
        let unparsed: Vec<&str> = with_duration
            .iter()
            .filter_map(|s| {
                let parsed = parse_duration(s.duration.as_deref());
                (!parsed.is_permanent && parsed.max_secs.is_none())
                    .then(|| s.duration.as_deref().unwrap_or(""))
            })
            .collect();
        let rate = 1.0 - (unparsed.len() as f64 / with_duration.len() as f64);
        println!(
            "duration strings: {}, unparsed: {} ({:.1}% parsed) -- e.g. {:?}",
            with_duration.len(),
            unparsed.len(),
            rate * 100.0,
            &unparsed[..unparsed.len().min(10)]
        );
        assert!(
            rate > 0.95,
            "expected the large majority to parse, got {:.1}%",
            rate * 100.0
        );
    }
}
