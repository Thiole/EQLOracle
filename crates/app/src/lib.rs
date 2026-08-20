//! Library face of the desktop shell -- every module `main.rs`'s binary
//! target needs, plus what nothing inside the running app needs but
//! something *outside* it does: `examples/dump_fixtures.rs` builds a real
//! `Ingest` against `fixtures/reference-slice.log` through this same
//! library to produce the UI's mock-IPC-harness JSON snapshots (see
//! `ui/tests/README.md` and `docs/ci.md`'s "mock IPC harness"), and any
//! future integration test wants the same access. Splitting this out
//! rather than duplicating `mod` declarations in both `main.rs` and here
//! is deliberate: two independent `mod ingest;` lines would each compile
//! `ingest.rs` into a *different* type (`main`'s own `Ingest` and this
//! crate's `Ingest` would not be the same type), so the modules live here,
//! exactly once, and `main.rs` reuses them via `use eqlp_app::*`.

pub mod aadata;
pub mod character;
pub mod classdata;
pub mod combat;
pub mod commands;
pub mod config;
pub mod debugview;
pub mod flavordata;
pub mod gearplanner;
pub mod history;
pub mod hpdata;
pub mod ingest;
pub mod inventory;
pub mod invocationdata;
pub mod itemdata;
pub mod manadata;
pub mod mapsdata;
pub mod mobalias;
pub mod monsterdata;
pub mod monsters;
pub mod notifications;
pub mod npcdata;
pub mod overview;
pub mod parser;
pub mod preferences;
pub mod progression;
pub mod settings;
pub mod skilldata;
pub mod spelldata;
pub mod spelleffect;
pub mod stancedata;
pub mod state;
pub mod tail_worker;
pub mod teleportdata;
pub mod zone;
pub mod zonedata;
