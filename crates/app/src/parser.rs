//! Wires the pure parser core (`eqlp-core`) into the live app.
//!
//! Only glue lives here: engine construction from the bundled pack, and the
//! per-file counters the UI shows. No classification logic -- that stays in
//! `eqlp-core`, per `docs/design/parsing.md`.

use eqlp_core::{engine_from_toml, Engine, Outcome, PackError};
use serde::Serialize;
use std::collections::BTreeMap;

/// The rule pack ships inside the binary. An installed desktop app has no
/// reliable working directory to find `packs/eql.toml` relative to, so the
/// pack is data baked in at compile time rather than a runtime file read --
/// still a plain TOML file, just resolved at build time instead of launch.
const PACK_TOML: &str = include_str!("../../../packs/eql.toml");

pub fn build_engine() -> Result<Engine, PackError> {
    engine_from_toml(&[PACK_TOML])
}

/// Running counters for whichever file is currently being tailed. Reset
/// whenever the tail target changes (new file, truncation, replacement) so
/// numbers always describe "this file", not "since the app opened".
///
/// Deliberately not `eqlp_core::Coverage`: that type also clusters unmatched
/// line shapes for the authoring workflow, an unbounded-growth structure
/// that is wrong for a tail meant to run for days. This is four counters and
/// a small fixed-cardinality kind map.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Counts {
    pub total: u64,
    pub matched: u64,
    pub unmatched: u64,
    pub headerless: u64,
    pub blank: u64,
    pub by_kind: BTreeMap<String, u64>,
}

impl Counts {
    pub fn record(&mut self, outcome: &Outcome, kind: Option<&str>) {
        self.total += 1;
        match outcome {
            Outcome::Matched(_) => {
                self.matched += 1;
                if let Some(k) = kind {
                    *self.by_kind.entry(k.to_string()).or_insert(0) += 1;
                }
            }
            Outcome::Unmatched { .. } => self.unmatched += 1,
            Outcome::Headerless { .. } => self.headerless += 1,
            Outcome::Blank => self.blank += 1,
        }
    }
}

/// One matched line, trimmed to what the live feed shows. Owned strings:
/// this crosses the Tauri IPC boundary, well past where the zero-copy
/// `Match` in `eqlp-core` is valid.
#[derive(Debug, Clone, Serialize)]
pub struct RecentLine {
    pub kind: String,
    pub rule_id: String,
    pub text: String,
}
