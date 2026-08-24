//! why: builds the `Engine` from the bundled pack for the live app

use eqlp_core::{engine_from_toml, Engine, PackError};

/// why: baked in at compile time -- an installed app has no reliable cwd
const PACK_TOML: &str = include_str!("../../../packs/eql.toml");

pub fn build_engine() -> Result<Engine, PackError> {
    engine_from_toml(&[PACK_TOML])
}
