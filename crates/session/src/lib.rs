//! Encounters, rolling DPS, time-to-kill. Stateful interpretation over the
//! event stream.
//!
//! Design notes: `docs/design/session.md`

pub mod allegiance;
pub mod cast;
pub mod classdetect;
pub mod context;
pub mod encounter;
pub mod graph;
pub mod group;
pub mod rolling;
pub mod timeline;

pub use allegiance::Allegiance;
pub use cast::{Outcome as CastOutcome, Resolution as CastResolution, Resolver as CastResolver};
pub use classdetect::Detector as ClassDetector;
pub use context::{Context, Sessions, Spans};
pub use encounter::{Encounter, EndReason, HpModel, Tracker, Ttk};
pub use graph::{is_pet_suffixed, Builder, Closed, EncId, Entities, Kind, Live, Policy};
pub use group::GroupTracker;
pub use rolling::Rolling;
pub use timeline::{series, Bucket, Cause, State, Timeline, Transition};

/// why: first char case-folded only -- fixes "an armadillo"/"An armadillo"
/// without merging distinct proper nouns like other mobs would need
pub(crate) fn fold_key(name: &str) -> String {
    let mut c = name.chars();
    match c.next() {
        Some(f) => f.to_lowercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}
