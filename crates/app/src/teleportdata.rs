//! Wires the scraped teleport-spell -> exact landing coordinate lookup
//! (`packs/teleport_landings.json`) into the live app, the same way
//! `classdata.rs` wires the spell -> class lookup in.
//!
//! Real motivation: the Maps module's "you are here" entrance guess for a
//! Wizard Translocate/Gate/Portal or Druid Circle/Ring landing used to be
//! a map-marker-label match ("does exactly one Wizard_Spire/Druid_Circle
//! marker exist on this zone's map") -- a guess, and for Druid specifically
//! an unverified one (no real map pack available to confirm the label
//! convention against). Unnecessary: eqlwiki states the *exact* (x, y, z)
//! landing coordinate for every one of these spells directly, via a
//! `{{SpellSlotRowSmart | 1 | Teleport group to X,Y,Z in [[Zone]] | ... }}`
//! row -- confirmed by fetching the live raw wikitext for four real spells
//! directly (North Karana Gate, North Karana Portal, Circle of Karana,
//! Translocate: North Karana), not assumed.
//!
//! `packs/teleport_landings.json` is generated, not hand-written -- built
//! by `~/eql/build_teleport_landings.py` from the raw scrape
//! (`~/eql/spells.json`), 103 entries as of the 2026-08-21 build (105
//! minus 2 real losses -- `level` addition below found two entries whose
//! own `classes` field came back empty in the raw scrape, so no level
//! requirement could be determined; see `TeleportLanding::level`'s own
//! doc). Re-run
//! that script whenever `spells.json` is refreshed. Keyed by the spell's
//! full name (not rank-stripped -- these destination-zone spells are
//! terminal, not part of a Roman-numeral rank family the way most spells
//! are, so no `base_spell_name` folding is needed or done here).
//!
//! This pack is the **sole source of truth** for "is this cast a real
//! fixed-destination teleport" -- deliberately not a name-shape heuristic
//! (`starts_with("Circle of ")` etc.) on its own, because that heuristic
//! has real false positives confirmed against actual slot data: "Circle of
//! Force"/"Circle of Summer"/"Circle of Winter" match the "Circle of X"
//! name shape but are damage-shield/resist buffs, not teleports, and have
//! no coordinate-shaped effect text at all -- naturally excluded here by
//! requiring a real parsed coordinate, not papered over with a denylist.
//! `landing_for` returning `None` means "not a known fixed-destination
//! teleport", full stop -- callers must not fall back to a name-shape
//! guess on top of this, that would reintroduce the exact bug this exists
//! to fix.
//!
//! Known, stated gap, not hidden: 8 name-shape matches during the build
//! are absent from this pack -- six for the reasons already documented
//! (three false positives above, `Iceclad Portal`/`Cazic Gate` missing
//! their own coordinates in the wiki's own effect text, `Ring of North
//! Karana` never surfaced in the `Category:Spells` scrape at all despite
//! a real page existing), plus two more (`Ring of Faydark`, `Thurgadin
//! Gate`) added when the `level` field below needed a real per-class
//! level the raw scrape's own `classes` field came back empty for. A cast
//! of one of these eight is invisible to the Maps module's entrance guess
//! entirely (no landing, and no marker-guess fallback either -- see this
//! module's own history for why that fallback was removed rather than
//! kept as a second, weaker tier).
//!
//! Coordinate space, stated not proven: these values are assumed to be in
//! the same space the game's own `/loc` command reports (matching how
//! `Ingest::last_loc`'s doc already reasons about that command, and the
//! same transform `MapViewer.svelte` applies to a real `/loc` reading
//! before plotting it) -- these wiki entries are almost certainly sourced
//! by a player typing `/loc` right after landing, not independently
//! measured some other way. **Not independently cross-verified against a
//! real `/loc` reading in the reference log**: checked directly -- of 115
//! real teleport-cast-then-zone-enter events in the reference log, zero
//! have a `/loc` reading before the next zone change, so there is no real
//! data point available in this sandbox to confirm the coordinate space
//! against. Stated as a real, checked limit, not silently assumed.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::OnceLock;

const TELEPORT_DATA_JSON: &str = include_str!("../../../packs/teleport_landings.json");

/// Which class's teleport spell this is -- decides which family of
/// landing-spot map marker the Maps module's "you are here" note names
/// (`Wizard_Spire` vs `Druid_Circle`/`Druid_Ring`/`Druid_Portal`), purely
/// for the note text shown to the user -- the actual plotted position
/// comes from `TeleportLanding::x`/`y`/`z`, not a marker lookup at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TeleportClass {
    Wizard,
    Druid,
    /// Not from this module's own static, wiki-scraped pack -- built at
    /// runtime for the one real teleport that genuinely has no fixed
    /// class or destination: `Origin` (see `Ingest::learned_origin`'s own
    /// doc). Every class can use it, so it deliberately isn't gated the
    /// way `Wizard`/`Druid` are wherever a `TeleportLanding`'s class gets
    /// checked against a player's own confirmed classes.
    Any,
}

impl TeleportClass {
    /// The full class name, matching how every other class-evidence
    /// consumer in this app names it (`classdetect`'s confirmed
    /// configurations, `packs/spell_classes.json`'s own `VALID_CLASSES`)
    /// -- used by `routing.rs` to check a teleport edge against a
    /// player's own confirmed class list.
    pub fn as_str(self) -> &'static str {
        match self {
            TeleportClass::Wizard => "Wizard",
            TeleportClass::Druid => "Druid",
            TeleportClass::Any => "Any",
        }
    }
}

/// One teleport spell's exact, wiki-confirmed landing spot. Coordinate
/// order/meaning matches `Ingest::last_loc`'s own `/loc`-space fields --
/// see this module's own top doc for the "assumed, not proven" caveat on
/// that.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TeleportLanding {
    pub class: TeleportClass,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    /// The destination zone, as `build_teleport_landings.py` scraped it
    /// out of the wiki's own `... in [[Zone]]`/`... in zone` slot text --
    /// added for `routing.rs`'s zone-graph (a teleport is a real
    /// zone-to-zone edge), the first consumer that's ever needed *which*
    /// zone a landing is in rather than just the landing point itself.
    /// **Not a clean, single format** -- confirmed directly against the
    /// real pack: sometimes the wiki's own display name (`"North Karana"`,
    /// `"Dreadlands"`), sometimes a bare map-shortname-looking string the
    /// wiki page left unlinked (`"butcher"`, `"commons"` -- these read
    /// exactly like `zonedata::map_shortnames` output, not coincidentally,
    /// since that's genuinely what the wiki editor typed). `routing.rs`
    /// resolves either shape against real `zonedata::Zone` data rather
    /// than assuming one format.
    pub zone: String,
    /// The minimum level `class` needs to have this spell -- straight
    /// from `spells.json`'s own real `classes: [{class, level}]` field
    /// (matched by class name, not just the first entry, since a spell
    /// can list several classes each at their own level). Used by
    /// `routing.rs` to filter which teleport shortcuts a *specific*
    /// player's assumed class/level combination could actually cast --
    /// see `routing::filter_castable`'s own doc. Two real entries
    /// (`"Ring of Faydark"`, `"Thurgadin Gate"`) are absent from the pack
    /// entirely because the raw scrape's own `classes` field came back
    /// empty for them -- a genuine upstream gap, not guessed around.
    pub level: u8,
}

#[derive(Deserialize)]
struct RawLanding {
    class: String,
    x: f64,
    y: f64,
    z: f64,
    zone: String,
    level: u8,
}

static TELEPORT_DATA: OnceLock<HashMap<String, TeleportLanding>> = OnceLock::new();

fn data() -> &'static HashMap<String, TeleportLanding> {
    TELEPORT_DATA.get_or_init(|| {
        let raw: HashMap<String, RawLanding> = serde_json::from_str(TELEPORT_DATA_JSON)
            .unwrap_or_else(|e| {
                // A malformed embedded file is a build-time data bug, not a
                // runtime condition to recover from gracefully -- same
                // stance `classdata.rs` takes on its own embedded pack.
                panic!("packs/teleport_landings.json failed to parse: {e}")
            });
        raw.into_iter()
            .map(|(name, r)| {
                let class = match r.class.as_str() {
                    "wizard" => TeleportClass::Wizard,
                    "druid" => TeleportClass::Druid,
                    other => {
                        panic!("packs/teleport_landings.json: unknown class {other:?} for {name:?}")
                    }
                };
                (
                    name,
                    TeleportLanding {
                        class,
                        x: r.x,
                        y: r.y,
                        z: r.z,
                        zone: r.zone,
                        level: r.level,
                    },
                )
            })
            .collect()
    })
}

/// The exact landing spot for a recognized teleport cast, by the spell's
/// full (un-rank-stripped) name -- `None` for anything not in the pack,
/// which includes both ordinary non-teleport spells and the six real,
/// documented gaps this module's own doc lists. See this module's own doc
/// for why this is the sole recognizer, not a name-shape heuristic.
pub fn landing_for(spell_name: &str) -> Option<TeleportLanding> {
    data().get(spell_name).cloned()
}

/// Every known teleport spell and its landing, spell name first -- used by
/// `routing.rs` to seed the zone-graph with teleport-shortcut edges (a
/// consumer `landing_for`'s single-lookup shape can't serve, since it
/// needs *all* of them, not one at a time).
pub fn all_landings() -> impl Iterator<Item = (&'static str, &'static TeleportLanding)> {
    data()
        .iter()
        .map(|(name, landing)| (name.as_str(), landing))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real data, cross-checked against a live wikitext fetch of
    /// North_Karana_Gate before this pack was built -- not a synthetic
    /// fixture.
    #[test]
    fn a_real_wizard_gate_spell_resolves_its_confirmed_landing() {
        let landing = landing_for("North Karana Gate").expect("known teleport spell");
        assert_eq!(landing.class, TeleportClass::Wizard);
        assert_eq!((landing.x, landing.y, landing.z), (-3685.0, 1209.0, -5.0));
        // Real level requirement, confirmed against the raw scrape's own
        // classes field directly (Wizard 18) -- not guessed.
        assert_eq!(landing.level, 18);
    }

    /// Same real-log cross-check for the Druid side.
    #[test]
    fn a_real_druid_circle_spell_resolves_its_confirmed_landing() {
        let landing = landing_for("Circle of Karana").expect("known teleport spell");
        assert_eq!(landing.class, TeleportClass::Druid);
        assert_eq!((landing.x, landing.y, landing.z), (-2706.0, -1494.0, -4.0));
    }

    /// The confirmed false-positive case this pack exists to exclude: a
    /// "Circle of X"-shaped name that is not actually a teleport.
    #[test]
    fn a_name_shape_false_positive_has_no_landing() {
        assert!(landing_for("Circle of Summer").is_none());
    }

    #[test]
    fn an_ordinary_non_teleport_spell_has_no_landing() {
        assert!(landing_for("Fireball").is_none());
    }
}
