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
/// why: one folding rule for entity names, shared -- monsterdata.rs
/// kept a byte-for-byte copy of this because it was crate-private
/// why: borrows when folding changes nothing, which is the overwhelming
/// case -- mob names already start lowercase. The owning version cost two
/// heap allocations to lowercase ONE character, on a function the damage
/// path reaches 6-10 times per event.
pub fn fold_key(name: &str) -> std::borrow::Cow<'_, str> {
    let mut c = name.chars();
    let Some(f) = c.next() else {
        return std::borrow::Cow::Borrowed("");
    };
    let mut low = f.to_lowercase();
    if low.next() == Some(f) && low.next().is_none() {
        return std::borrow::Cow::Borrowed(name);
    }
    let mut out = String::with_capacity(name.len());
    out.extend(f.to_lowercase());
    out.push_str(c.as_str());
    std::borrow::Cow::Owned(out)
}

/// why: the comparison sites never needed a key at all -- `fold_key(a) ==
/// fold_key(b)` allocated twice to answer a question about two &str
pub fn fold_eq(a: &str, b: &str) -> bool {
    let (mut x, mut y) = (a.chars(), b.chars());
    match (x.next(), y.next()) {
        (Some(p), Some(q)) => p.to_lowercase().eq(q.to_lowercase()) && x.as_str() == y.as_str(),
        (None, None) => true,
        _ => false,
    }
}
