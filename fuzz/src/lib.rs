//! Shared setup for the fuzz targets that need a real `Engine` --
//! currently just `classify.rs`. Mirrors `eqlp_app::parser::build_engine`
//! exactly (same pack, same `engine_from_toml` call) rather than
//! depending on `eqlp-app` itself, which would pull in Tauri and its own
//! GTK/glib system dependencies as a fuzz-build requirement for no
//! reason -- fuzzing only needs the pure parser core.

use eqlp_core::{engine_from_toml, Engine};

const PACK_TOML: &str = include_str!("../../packs/eql.toml");

pub fn engine() -> Engine {
    engine_from_toml(&[PACK_TOML])
        .expect("packs/eql.toml must build -- same pack the real app ships")
}
