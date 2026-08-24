//! why: teleport-spell -> exact landing coordinate, baked in like `classdata.rs`
//!
//! Replaces the Maps module's old map-marker-label guess -- eqlwiki
//! states the exact (x,y,z) landing directly, confirmed against 4 real
//! spells' raw wikitext. Sole source of truth for "is this a real
//! fixed-destination teleport" -- deliberately not a name-shape heuristic
//! (confirmed false positives: "Circle of Summer/Winter/Force" are buffs,
//! not teleports). `None` means not a known teleport, full stop -- no
//! name-shape fallback on top.
//!
//! Known gap: 8 name-shape matches absent (3 false positives above, 2
//! missing wiki coordinates, 1 missing from the scrape, 2 missing level
//! data). Coordinate space assumed to match `/loc`, not independently
//! cross-verified -- zero real teleport-then-`/loc` events exist in the
//! reference log to confirm against; stated as a real, checked limit.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::OnceLock;

const TELEPORT_DATA_JSON: &str = include_str!("../../../packs/teleport_landings.json");

/// why: decides the note-text marker family shown; plotted position is
/// always `TeleportLanding::x`/`y`/`z`, never a marker lookup
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TeleportClass {
    Wizard,
    Druid,
    /// why: built at runtime for `Origin`, the one every-class teleport --
    /// not gated by confirmed classes the way Wizard/Druid are
    Any,
}

impl TeleportClass {
    /// why: full class name, matches every other class-evidence consumer
    pub fn as_str(self) -> &'static str {
        match self {
            TeleportClass::Wizard => "Wizard",
            TeleportClass::Druid => "Druid",
            TeleportClass::Any => "Any",
        }
    }
}

/// why: exact wiki-confirmed landing; coordinate order matches `Ingest::last_loc`
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TeleportLanding {
    pub class: TeleportClass,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    /// why: destination zone for `routing.rs`'s graph; not one clean
    /// format -- sometimes display name, sometimes a bare map shortname
    pub zone: String,
    /// why: min level for `class`, from `spells.json`; feeds
    /// `routing::filter_castable`. 2 entries missing (empty upstream data)
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
                // why: malformed embedded data is a build bug, fail loud
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

/// why: exact landing by full spell name; None for anything not in the pack
pub fn landing_for(spell_name: &str) -> Option<TeleportLanding> {
    data().get(spell_name).cloned()
}

/// why: every landing at once, for `routing.rs` to seed the zone-graph
pub fn all_landings() -> impl Iterator<Item = (&'static str, &'static TeleportLanding)> {
    data()
        .iter()
        .map(|(name, landing)| (name.as_str(), landing))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// why: real data, cross-checked against a live wikitext fetch
    #[test]
    fn a_real_wizard_gate_spell_resolves_its_confirmed_landing() {
        let landing = landing_for("North Karana Gate").expect("known teleport spell");
        assert_eq!(landing.class, TeleportClass::Wizard);
        assert_eq!((landing.x, landing.y, landing.z), (-3685.0, 1209.0, -5.0));
        // why: real level requirement confirmed against raw scrape, not guessed
        assert_eq!(landing.level, 18);
    }

    /// why: same real-log cross-check for the Druid side
    #[test]
    fn a_real_druid_circle_spell_resolves_its_confirmed_landing() {
        let landing = landing_for("Circle of Karana").expect("known teleport spell");
        assert_eq!(landing.class, TeleportClass::Druid);
        assert_eq!((landing.x, landing.y, landing.z), (-2706.0, -1494.0, -4.0));
    }

    /// why: confirmed false positive this pack excludes -- name-shape, not a teleport
    #[test]
    fn a_name_shape_false_positive_has_no_landing() {
        assert!(landing_for("Circle of Summer").is_none());
    }

    #[test]
    fn an_ordinary_non_teleport_spell_has_no_landing() {
        assert!(landing_for("Fireball").is_none());
    }
}
