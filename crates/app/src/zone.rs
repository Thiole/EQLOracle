//! Zone difficulty tier, parsed from the zone name itself, and matching a
//! raw `zone.enter` label against the wiki's own zone name.
//!
//! EQL carries no dedicated "difficulty" log line -- the game bakes the
//! tier straight into the zone name `zone.enter` already captures. The base
//! zone is tier 0 ("Befallen"); tiers 1-4 append a fixed " <N> (<Word>)"
//! suffix -- "Befallen 1 (Awakened)", "...2 (Adaptive)", "...3 (Fused)",
//! "...4 (Refined)". Confirmed against every zone line in the reference
//! log (120 distinct labels, 41 of them tiered): the digit and the
//! parenthesised word always agree, and this holds across "- Group" raid
//! variants too ("The Plane of Fear - Group 4 (Refined)") -- matching the
//! fixed suffix at the end is sufficient; what precedes it doesn't matter.
//!
//! `zone_matches` is what anything cross-referencing a log zone against
//! `zonedata::Zone::name` should actually call -- `zone_tier`/`zone_key`
//! alone handle the *formatting* gap (tier suffix, raid marker, a leading
//! "The"), but checked against the same real reference log, 42 of its 120
//! distinct labels still didn't resolve after that: the wiki's own
//! zone-guide titles use a different naming convention outright for a
//! real chunk of zones, not just different formatting. `ZONE_ALIASES`
//! closes that gap for the ones a live wiki check could actually confirm.

/// Suffix -> tier. Order doesn't matter -- the four suffixes share no
/// common tail, so at most one can ever match.
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

/// A zone label's own comparison key -- for matching a raw `zone.enter`
/// string against the wiki's own canonical zone name (`zonedata::Zone::
/// name`), which was scraped as its own independent pass and doesn't
/// necessarily carry either the difficulty/raid suffix the log always
/// states or the leading "The" some zone names take and others don't.
/// Confirmed against real data (`packs/zones.json`): "Plane of Fear" is
/// the wiki's own name for what the log calls "The Plane of Fear - Group
/// 4 (Refined)", but "The Feerrott" -- a *different* zone -- really does
/// keep "The" in the wiki's own title too. Since which zones keep it is
/// inconsistent, this only produces a fair comparison when *both* sides
/// of an equality check go through it -- one raw string against one
/// already-stripped name would just trade one mismatch for another.
///
/// Case is left alone; compare the results with `eq_ignore_ascii_case`,
/// not here, so this stays a pure substring op with no allocation.
pub fn zone_key(raw: &str) -> &str {
    let (base, _) = zone_tier(raw);
    // "- Group" (multiplayer) and "- Solo" (solo-instanced) are the game's
    // own two raid-instance-type markers -- eqlwiki's own "Solo vs.
    // Multiplayer" section on a raid zone's page confirms both are real,
    // named options for the same instance, not two different zones. Only
    // "- Group" was handled here until a real "- Solo" label (`The
    // Permafrost Caverns - Solo 4 (Refined)`) showed up and silently
    // missed both this stripping *and* the alias below it exists for --
    // the alias itself was never reachable, so this and that entry are
    // one bug, not two.
    let base = base
        .strip_suffix(" - Group")
        .or_else(|| base.strip_suffix(" - Solo"))
        .unwrap_or(base);
    base.strip_prefix("The ").unwrap_or(base)
}

/// `zone_key`'s own remaining gap, checked against the real reference log
/// (120 distinct `zone.enter` labels): after tier/raid/"The" stripping,
/// 42 of them *still* didn't match any `packs/zones.json` name -- the
/// wiki's zone-guide page titles use a genuinely different naming
/// convention from the game client's for a real chunk of zones, not just
/// a formatting difference `zone_key` can strip its way past. Confirmed
/// one by one against the live wiki (its own redirect table, where one
/// exists, plus manual review otherwise -- not guessed): district zones
/// collapse to their parent ("Neriak - Foreign Quarter" -> "Neriak"),
/// some pages are titled shorter than the in-game name ("Clan Crushbone"
/// -> "Crushbone"), and a few are just differently worded entirely
/// ("Kerra Isle" -> "Kerra Island", "The Lair of the Splitpaw" ->
/// "Splitpaw Lair"). Keyed and valued the same way `zone_key`'s own output
/// is compared -- lowercase via `eq_ignore_ascii_case` at the call site,
/// not here.
///
/// `("Ruins of Old Paineel", "Hole")` was previously left out on the
/// assumption there was no wiki zone-guide page for it under any title --
/// wrong, confirmed by fetching eqlwiki directly: "The Hole" opens with
/// "officially known as The Ruins of Old Paineel", and (like "Permafrost"
/// below) documents its raid instance on the *same* page as the ordinary
/// leveling dungeon rather than a separate one -- there was never a
/// missing page, only a missing alias entry, the same "- Solo"/"- Group"
/// stripping gap `zone_key`'s own doc now covers meant it would have gone
/// unreached anyway. Still correctly does *not* alias to "Paineel" (the
/// peaceful city) -- a different real zone, not this one under another
/// name; see this file's own tests.
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
    ("Permafrost Keep", "Permafrost"),
    ("Permafrost Caverns", "Permafrost"),
    // "Hole", not "The Hole" -- zone_matches compares an alias's RHS
    // as-is against zone_key(wiki_name), which strips wiki_name's own
    // leading "The " (the wiki's actual page name is "The Hole"); every
    // other alias RHS in this table is already written pre-stripped for
    // the same reason.
    ("Ruins of Old Paineel", "Hole"),
    ("Temple of Cazic-Thule", "Cazic Thule (Zone)"),
    ("City of Guk", "Upper Guk"),
    ("Lair of the Splitpaw", "Splitpaw Lair"),
    ("Liberated Citadel of Runnyeye", "Runnyeye"),
    ("Qeynos Aqueduct System", "Qeynos Aqueducts"),
    ("Ruins of Old Guk", "Lower Guk"),
    ("Southern Plains of Karana", "Southern Karana"),
    ("Western Plains of Karana", "Western Karana"),
    // Word order, not just formatting -- the log says "The Castle of
    // Mistmoore", the wiki's own zone-guide page is titled "Mistmoore
    // Castle". zone_key strips the leading "The ", leaving "Castle of
    // Mistmoore", which still doesn't match without this entry.
    ("Castle of Mistmoore", "Mistmoore Castle"),
];

/// Whether `raw` (a `zone.enter` label straight off the log, tier suffix
/// and all) and `wiki_name` (`zonedata::Zone::name`) refer to the same
/// zone. The one function `combat::list_zone_encounters` and anything
/// else cross-referencing a log zone against the wiki should call --
/// `zone_key` alone isn't enough (see `ZONE_ALIASES`' own doc for why),
/// and inlining the alias lookup at every call site would just be this
/// function copied out.
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
        // The exact real-data case that motivated this: the log states a
        // raid instance's tier and "- Group" every time, the wiki's own
        // zone page carries neither.
        assert_eq!(
            zone_key("The Plane of Fear - Group 4 (Refined)"),
            "Plane of Fear"
        );
        assert_eq!(zone_key("Plane of Fear"), "Plane of Fear");
        // A zone whose wiki title *does* keep "The" -- stripped from both
        // sides alike, so the two still compare equal to each other, just
        // not to the "Plane of Fear" case above.
        assert_eq!(zone_key("The Feerrott"), "Feerrott");
        // Untiered, no "The", nothing to strip -- a no-op, not a special case.
        assert_eq!(zone_key("New Sebilis Expedition"), "New Sebilis Expedition");
        // "- Solo", not just "- Group" -- a real label
        // (`The Permafrost Caverns - Solo 4 (Refined)`) that used to fall
        // through this stripping entirely, taking its own (already
        // correct) ZONE_ALIASES entry down with it.
        assert_eq!(
            zone_key("The Permafrost Caverns - Solo 4 (Refined)"),
            "Permafrost Caverns"
        );
    }

    #[test]
    fn zone_matches_covers_the_real_reference_log() {
        // A sample of the 24 real mismatches ZONE_ALIASES exists for --
        // stripping alone (zone_key) can't close these, only a lookup can.
        assert!(zone_matches("Clan Crushbone", "Crushbone"));
        assert!(zone_matches("North Qeynos", "Qeynos"));
        assert!(zone_matches("South Qeynos", "Qeynos"));
        assert!(zone_matches("Neriak - Foreign Quarter", "Neriak"));
        assert!(zone_matches("The Ruins of Old Guk", "Lower Guk"));
        assert!(zone_matches("The City of Guk", "Upper Guk"));
        assert!(zone_matches("Temple of Cazic-Thule", "Cazic Thule (Zone)"));
        assert!(zone_matches("Kerra Isle", "Kerra Island"));
        assert!(zone_matches("The Castle of Mistmoore", "Mistmoore Castle"));
        // Ordinary zone_key cases still work through the same function.
        assert!(zone_matches(
            "The Plane of Fear - Group 4 (Refined)",
            "Plane of Fear"
        ));
        assert!(zone_matches(
            "New Sebilis Expedition",
            "New Sebilis Expedition"
        ));
        // Two different real zones must never read as a match.
        assert!(!zone_matches(
            "Clan Crushbone",
            "Crushbone Tunnel Excavation"
        ));
        assert!(!zone_matches("North Qeynos", "Qeynos Hills"));
        // "The Ruins of Old Paineel" is real (eqlwiki: "The Hole --
        // officially known as The Ruins of Old Paineel") -- must resolve
        // to "The Hole", and must still NOT match "Paineel", a genuinely
        // different zone (the peaceful city) sharing only a root word.
        assert!(zone_matches("The Ruins of Old Paineel", "The Hole"));
        assert!(!zone_matches("The Ruins of Old Paineel", "Paineel"));
        // The raid instance's own two real suffix forms, both confirmed
        // in a real user's own live game (only "- Group" was ever in the
        // bundled reference log) -- both resolve the same as the plain
        // leveling-dungeon zone, since eqlwiki documents raid and
        // overworld content on the one page.
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
