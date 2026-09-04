//! Where log bytes come from and what time it is while they arrive.
//!
//! Design notes: `docs/design/sources.md`

pub mod clock;
pub mod replay;
pub mod tail;

pub use clock::{Millis, SystemClock, VirtualClock};
pub use replay::{Replay, Speed};
pub use tail::{identity_from_filename, newest_log_in, Tail, TailEvent};

/// why: 100ms -- line latency reads as instant on the live meter.
/// Cost per idle poll is one fs::metadata call (no read when size is
/// unchanged), so 10/s is noise. Not configurable to zero on purpose.
pub const POLL_MS: u64 = 100;
pub const MIN_POLL_MS: u64 = 50;
