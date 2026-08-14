//! Wires the pure parser core (`eqlp-core`) into the live app: builds an
//! `Engine` from the bundled pack. Everything downstream of classification
//! (counters, the store, encounters) lives in `ingest`.

use eqlp_core::{engine_from_toml, Engine, PackError};

/// The rule pack ships inside the binary. An installed desktop app has no
/// reliable working directory to find `packs/eql.toml` relative to, so the
/// pack is data baked in at compile time rather than a runtime file read --
/// still a plain TOML file, just resolved at build time instead of launch.
const PACK_TOML: &str = include_str!("../../../packs/eql.toml");

pub fn build_engine() -> Result<Engine, PackError> {
    engine_from_toml(&[PACK_TOML])
}
