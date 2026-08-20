//! Wires the scraped spell -> class lookup (`packs/spell_classes.json`) into
//! the live app, the same way `parser.rs` wires the rule pack in.
//!
//! Same reasoning as the rule pack: an installed desktop app has no
//! reliable working directory to read a data file from at launch, so this
//! ships baked into the binary at compile time instead.
//!
//! `packs/spell_classes.json` is generated, not hand-written -- built by
//! `~/eql/build_spell_classes.py` directly from the raw scrape
//! (`~/eql/spells.json`, itself produced by `scrape_eqlwiki_spells.py`
//! exhaustively walking `Category:Spells` via the wiki API: 1,928 spell
//! pages as of the 2026-08-14 scrape). Re-run `build_spell_classes.py`
//! whenever `spells.json` is refreshed to pick up wiki changes; this file
//! is a snapshot, not derived at build time.
//!
//! Keyed by *base* (rank-stripped) name, not the wiki page title -- a rank
//! variant (`Elemental Maelstrom X`) is its own wiki page, but
//! `Ingest::base_spell_name` strips the rank before ever looking a name
//! up, so `build_spell_classes.py` merges every rank variant's class list
//! into one base-name entry using a line-for-line match of that same
//! stripping logic (including its `PROTECTED_SPELL_NAMES` exceptions) --
//! 1,431 base names as of the same scrape.
//!
//! Two things the build script filters, confirmed against the raw data
//! rather than assumed: an empty `classes` list (AA-adjacent effects
//! miscategorised under `Category:Spells`, mob-only spells) is dropped
//! rather than kept as a false "no eligible class" signal; a `classes`
//! entry that isn't one of the game's real class names (the wiki
//! template's own `classes=` field is occasionally a stray link elsewhere
//! on the page instead -- `"a ghoul"`, `"Spirit of Inferno Strike"`, seen
//! on live pages, not a hypothetical) is dropped the same way, and
//! `"Shadowknight"`/`"Shadow Knight"` (two spellings for the same class
//! across different wiki pages) are folded into one before either ever
//! reaches this lookup.
//!
//! Four entries are hand-verified exceptions layered on top by the build
//! script, not part of the scrape: `Harm Touch` lives under a
//! `Skill:`-prefixed wiki title (a redirect, outside `Category:Spells`
//! entirely), and `Leech`/`Malaisement`/`Blast of Cold` are real spells
//! with real class data that are simply absent from the `Category:Spells`
//! enumeration for reasons not otherwise investigated -- each confirmed
//! individually against eqlwiki before being added, not guessed.
//!
//! `Origin` -- the other `Skill:`-prefixed redirect investigated alongside
//! `Harm Touch` -- is deliberately *not* one of the four: it's an AA
//! available to every class (a teleport-to-bind-point on a cooldown, no
//! `classes=` field at all), so adding it here would manufacture false
//! class evidence for whichever class happened to be active when a player
//! used it, not real evidence. Its absence from `packs/spell_classes.json`
//! is confirmed correct, not an oversight -- see eqlwiki's `Skill_Origin`
//! page.

use std::collections::HashMap;
use std::sync::OnceLock;

const CLASS_DATA_JSON: &str = include_str!("../../../packs/spell_classes.json");

static CLASS_DATA: OnceLock<HashMap<String, Vec<String>>> = OnceLock::new();

/// Classes that know `base_spell_name`, or an empty slice if the spell
/// isn't in the lookup at all (not "no eligible class" -- just unknown).
/// Parses the embedded JSON once, on first use.
pub fn classes_for(base_spell_name: &str) -> &'static [String] {
    let map = CLASS_DATA.get_or_init(|| {
        serde_json::from_str(CLASS_DATA_JSON).unwrap_or_else(|e| {
            // A malformed embedded file is a build-time data bug, not a
            // runtime condition to recover from gracefully -- same stance
            // as a bad rule pack failing `build_engine`. Loud and immediate
            // beats a silently empty class detector.
            panic!("packs/spell_classes.json failed to parse: {e}")
        })
    });
    map.get(base_spell_name)
        .map(|v| v.as_slice())
        .unwrap_or(&[])
}
