//! Where log bytes come from and what time it is while they arrive.
//!
//! Design notes: `docs/design/sources.md`

pub mod clock;
pub mod replay;
pub mod tail;

pub use clock::{Clock, Millis, SystemClock, VirtualClock};
pub use replay::{Replay, Speed};
pub use tail::{identity_from_filename, newest_log_in, Tail, TailEvent};

/// Poll interval for live tailing.
///
/// 250 ms is under the threshold where a combat meter feels laggy, and costs
/// four `stat` calls a second. Deliberately not configurable down to zero:
/// someone will set it to 1 ms and blame the app for their framerate.
pub const POLL_MS: u64 = 250;
pub const MIN_POLL_MS: u64 = 50;
