//! Encounters, rolling DPS, time-to-kill. Stateful interpretation over the
//! event stream.
//!
//! Design notes: `docs/design/session.md`

pub mod encounter;
pub mod context;
pub mod graph;
pub mod timeline;
pub mod rolling;

pub use timeline::{series, Bucket, Cause, State, Timeline, Transition};
pub use context::{Context, Sessions, Spans};
pub use graph::{Builder, Closed, EncId, Entities, Kind, Live, Policy};
pub use encounter::{EndReason, Encounter, HpModel, Tracker, Ttk};
pub use rolling::Rolling;
