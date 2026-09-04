//! why: zone difficulty tier from the zone name itself, and log-vs-wiki matching
//!
//! No dedicated difficulty log line -- the tier's baked into the zone
//! name. Tiers 1-4 append " <N> (<Word>)" (confirmed across 120 real
//! zone lines, digit and word always agree). `zone_matches` is what
//! callers should use -- `zone_tier`/`zone_key` alone only fix
//! formatting; 42 of 120 real labels still needed `ZONE_ALIASES` for a
//! genuinely different wiki naming convention.

/// why: suffix -> tier; order doesn't matter, no shared tail among them
const TIER_SUFFIXES: &[(&str, u8)] = &[
    (" 1 (Awakened)", 1),
    (" 2 (Adaptive)", 2),
    (" 3 (Fused)", 3),
    (" 4 (Refined)", 4),
];

/// Splits a zone label into its base name and difficulty tier (0-4). A
/// label with none of the four recognised suffixes reads as tier 0 --
/// correct both for an ordinary untiered zone and for tier 0 itself, which
/// carries no suffix at all.
pub fn zone_tier(zone: &str) -> (&str, u8) {
    for (suffix, tier) in TIER_SUFFIXES {
        if let Some(base) = zone.strip_suffix(suffix) {
            return (base, *tier);
        }
    }
    (zone, 0)
}

/// why: comparison key for log zone vs wiki name; "The" prefix is
/// inconsistent across zones, so both sides must go through this to be
/// a fair comparison. Case left alone -- compare with `eq_ignore_ascii_case`.
pub fn zone_key(raw: &str) -> &str {
    let (base, _) = zone_tier(raw);
    // why: "- Group"/"- Solo" are the game's two raid-instance markers,
    // same instance not different zones; "- Solo" once fell through this
    let base = base
        .strip_suffix(" - Group")
        .or_else(|| base.strip_suffix(" - Solo"))
        .unwrap_or(base);
    base.strip_prefix("The ").unwrap_or(base)
}

/// why: `zone_key`'s remaining gap -- 42 of 120 real labels still didn't
/// match after stripping, confirmed one by one against the live wiki:
/// districts collapse to parent, some pages shorter, some differently
/// worded. Compared lowercase at the call site, not here.
///
/// "Ruins of Old Paineel" -> "Hole": confirmed via eqlwiki, "The Hole"
/// is "officially known as The Ruins of Old Paineel" -- not a missing
/// page, a missing alias. Deliberately does NOT alias to "Paineel", a
/// different real zone.
const ZONE_ALIASES: &[(&str, &str)] = &[
    ("Clan Crushbone", "Crushbone"),
    ("East Freeport", "Freeport"),
    ("West Freeport", "Freeport"),
    ("Erudin Palace", "Erudin"),
    ("EverQuest Legends Tutorial", "Tutorial Zone"),
    ("Kerra Isle", "Kerra Island"),
    ("Neriak - Commons", "Neriak"),
    ("Neriak - Foreign Quarter", "Neriak"),
    ("Neriak - Third Gate", "Neriak"),
    ("North Kaladim", "Kaladim"),
    ("South Kaladim", "Kaladim"),
    ("North Qeynos", "Qeynos"),
    ("South Qeynos", "Qeynos"),
    ("Northern Felwithe", "Felwithe"),
    ("Southern Felwithe", "Felwithe"),
    ("Permafrost Keep", "Permafrost"),
    ("Permafrost Caverns", "Permafrost"),
    // why: "Hole" not "The Hole" -- RHS is compared pre-stripped, like every entry here
    ("Ruins of Old Paineel", "Hole"),
    ("Temple of Cazic-Thule", "Cazic Thule (Zone)"),
    ("City of Guk", "Upper Guk"),
    ("Lair of the Splitpaw", "Splitpaw Lair"),
    ("Liberated Citadel of Runnyeye", "Runnyeye"),
    ("Qeynos Aqueduct System", "Qeynos Aqueducts"),
    ("Ruins of Old Guk", "Lower Guk"),
    ("Southern Plains of Karana", "Southern Karana"),
    ("Western Plains of Karana", "Western Karana"),
    // why: word order, not formatting -- "Castle of Mistmoore" vs "Mistmoore Castle"
    ("Castle of Mistmoore", "Mistmoore Castle"),
];

/// why: the one function anything cross-referencing log zone vs wiki should call
pub fn zone_matches(raw: &str, wiki_name: &str) -> bool {
    let key = zone_key(raw);
    let resolved = ZONE_ALIASES
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map_or(key, |(_, v)| v);
    resolved.eq_ignore_ascii_case(zone_key(wiki_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_known_suffixes() {
        assert_eq!(zone_tier("Befallen"), ("Befallen", 0));
        assert_eq!(zone_tier("Befallen 1 (Awakened)"), ("Befallen", 1));
        assert_eq!(
            zone_tier("Nagafen's Lair 2 (Adaptive)"),
            ("Nagafen's Lair", 2)
        );
        assert_eq!(
            zone_tier("The Lair of the Splitpaw 3 (Fused)"),
            ("The Lair of the Splitpaw", 3)
        );
        assert_eq!(
            zone_tier("West Commonlands 4 (Refined)"),
            ("West Commonlands", 4)
        );
        assert_eq!(
            zone_tier("The Plane of Fear - Group 4 (Refined)"),
            ("The Plane of Fear - Group", 4)
        );
    }

    #[test]
    fn unrecognised_suffix_reads_as_tier_zero() {
        assert_eq!(
            zone_tier("An area where levitation effects do not function"),
            ("An area where levitation effects do not function", 0)
        );
    }

    #[test]
    fn zone_key_matches_raw_log_label_to_wiki_name() {
        // why: real-data case -- log always states tier + "- Group", wiki page carries neither
        assert_eq!(
            zone_key("The Plane of Fear - Group 4 (Refined)"),
            "Plane of Fear"
        );
        assert_eq!(zone_key("Plane of Fear"), "Plane of Fear");
        // why: wiki title keeps "The" too -- stripped both sides, still equal
        assert_eq!(zone_key("The Feerrott"), "Feerrott");
        // why: untiered, no "The", nothing to strip -- a no-op
        assert_eq!(zone_key("New Sebilis Expedition"), "New Sebilis Expedition");
        // why: "- Solo" real label that used to fall through this stripping
        assert_eq!(
            zone_key("The Permafrost Caverns - Solo 4 (Refined)"),
            "Permafrost Caverns"
        );
    }

    #[test]
    fn zone_matches_covers_the_real_reference_log() {
        // why: sample of 24 real mismatches ZONE_ALIASES closes, stripping alone can't
        assert!(zone_matches("Clan Crushbone", "Crushbone"));
        assert!(zone_matches("North Qeynos", "Qeynos"));
        assert!(zone_matches("South Qeynos", "Qeynos"));
        assert!(zone_matches("Neriak - Foreign Quarter", "Neriak"));
        assert!(zone_matches("The Ruins of Old Guk", "Lower Guk"));
        assert!(zone_matches("The City of Guk", "Upper Guk"));
        assert!(zone_matches("Temple of Cazic-Thule", "Cazic Thule (Zone)"));
        assert!(zone_matches("Kerra Isle", "Kerra Island"));
        assert!(zone_matches("The Castle of Mistmoore", "Mistmoore Castle"));
        // why: ordinary zone_key cases still work through the same function
        assert!(zone_matches(
            "The Plane of Fear - Group 4 (Refined)",
            "Plane of Fear"
        ));
        assert!(zone_matches(
            "New Sebilis Expedition",
            "New Sebilis Expedition"
        ));
        // why: two different real zones must never read as a match
        assert!(!zone_matches(
            "Clan Crushbone",
            "Crushbone Tunnel Excavation"
        ));
        assert!(!zone_matches("North Qeynos", "Qeynos Hills"));
        // why: "The Hole" officially known as "Ruins of Old Paineel", but
        // must not match "Paineel", a genuinely different zone
        assert!(zone_matches("The Ruins of Old Paineel", "The Hole"));
        assert!(!zone_matches("The Ruins of Old Paineel", "Paineel"));
        // why: both real raid suffix forms resolve to the same overworld zone
        assert!(zone_matches(
            "The Ruins of Old Paineel - Solo 4 (Refined)",
            "The Hole"
        ));
        assert!(zone_matches("The Ruins of Old Paineel - Group", "The Hole"));
        assert!(zone_matches(
            "The Permafrost Caverns - Solo 4 (Refined)",
            "Permafrost"
        ));
    }
}
