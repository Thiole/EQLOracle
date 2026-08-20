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
pub mod rolling;
pub mod timeline;

pub use allegiance::Allegiance;
pub use cast::{Outcome as CastOutcome, Resolution as CastResolution, Resolver as CastResolver};
pub use classdetect::Detector as ClassDetector;
pub use context::{Context, Sessions, Spans};
pub use encounter::{Encounter, EndReason, HpModel, Tracker, Ttk};
pub use graph::{Builder, Closed, EncId, Entities, Kind, Live, Policy};
pub use rolling::Rolling;
pub use timeline::{series, Bucket, Cause, State, Timeline, Transition};

/// Entity-name key: first character case-folded, rest preserved.
///
/// The log capitalises a name at sentence start and not mid-sentence -- the
/// same mob is `an armadillo` in "You hit an armadillo..." and
/// `An armadillo` in "An armadillo has been slain by...". Comparing names
/// raw silently fails to link the two, which is exactly the bug this crate's
/// design notes describe (`docs/design/session.md`, "Case folding"): 511
/// deaths closing only 114 fights before the fold, 450 after.
///
/// Only the first character folds. Lowercasing the whole name would merge
/// genuinely distinct targets -- proper nouns carry meaning (`a gnoll` and
/// `Gnoll Commander` are different mobs).
pub(crate) fn fold_key(name: &str) -> String {
    let mut c = name.chars();
    match c.next() {
        Some(f) => f.to_lowercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}
