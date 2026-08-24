//! Where log bytes come from and what time it is while they arrive.
//!
//! Design notes: `docs/design/sources.md`

pub mod clock;
pub mod replay;
pub mod tail;

pub use clock::{Clock, Millis, SystemClock, VirtualClock};
pub use replay::{Replay, Speed};
pub use tail::{identity_from_filename, newest_log_in, Tail, TailEvent};

/// why: below laggy-meter threshold, not configurable to zero on purpose
pub const POLL_MS: u64 = 250;
pub const MIN_POLL_MS: u64 = 50;
