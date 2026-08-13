//! Replaying a recorded log against a `VirtualClock`.
//!
//! Design notes: `docs/design/sources.md`

use crate::clock::{Clock, Millis, VirtualClock};
use eqlp_core::header::{BracketCtime, HeaderParser};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Speed {
    /// As fast as the CPU allows; virtual time still tracks the log.
    Instant,
    /// One virtual second per wall second.
    Realtime,
    /// `factor`× realtime. 0.5 is half speed.
    Scaled(f64),
}

impl Speed {
    /// How long to actually sleep for a given gap in log time.
    pub fn wall_delay_ms(&self, log_gap_ms: Millis) -> Millis {
        match self {
            Speed::Instant => 0,
            Speed::Realtime => log_gap_ms.max(0),
            Speed::Scaled(f) if *f > 0.0 => ((log_gap_ms.max(0) as f64) / f) as Millis,
            Speed::Scaled(_) => 0,
        }
    }
}

/// Drives a `VirtualClock` from timestamps in a log buffer. Does no I/O.
pub struct Replay<'a> {
    buf: &'a [u8],
    pos: usize,
    header: BracketCtime,
    speed: Speed,
    last_log_ms: Option<Millis>,
    pub lines_emitted: u64,
    /// Lines with no parseable timestamp. Still emitted.
    pub undated_lines: u64,
}

impl<'a> Replay<'a> {
    pub fn new(buf: &'a [u8], speed: Speed) -> Self {
        Replay {
            buf,
            pos: 0,
            header: BracketCtime,
            speed,
            last_log_ms: None,
            lines_emitted: 0,
            undated_lines: 0,
        }
    }

    /// Timestamp of the first dated line, for seeding the clock.
    pub fn first_timestamp_ms(&self) -> Option<Millis> {
        eqlp_core::frame::lines(self.buf)
            .filter_map(|l| self.header.parse(l).map(|(ts, _)| ts.0 * 1000))
            .next()
    }

    /// Advance one line. Returns the delay the caller should sleep before the
    /// next step; the caller owns sleeping.
    pub fn step(&mut self, clock: &VirtualClock, mut sink: impl FnMut(&[u8])) -> Option<Millis> {
        let line = self.next_line()?;

        let stamp = self.header.parse(line).map(|(ts, _)| ts.0 * 1000);
        let delay = match stamp {
            Some(ms) => {
                let gap = self.last_log_ms.map(|p| ms - p).unwrap_or(0);
                self.last_log_ms = Some(ms);
                clock.set_at_least(ms);
                self.speed.wall_delay_ms(gap)
            }
            None => {
                self.undated_lines += 1;
                0
            }
        };

        self.lines_emitted += 1;
        sink(line);
        Some(delay)
    }

    /// Run to completion. In `Instant` mode this is the history-backfill path.
    pub fn run_to_end(&mut self, clock: &VirtualClock, mut sink: impl FnMut(&[u8])) {
        while self.step(clock, &mut sink).is_some() {}
    }

    /// Replay until virtual time reaches `until_ms`. Returns false at EOF.
    /// This is the primitive a test uses: "advance to t, then assert".
    pub fn run_until(
        &mut self,
        clock: &VirtualClock,
        until_ms: Millis,
        mut sink: impl FnMut(&[u8]),
    ) -> bool {
        while clock.now_ms() < until_ms {
            // Peek: do not consume a line that belongs after the boundary, or
            // the caller's assertion would see an event from the future.
            match self.peek_timestamp_ms() {
                Some(next) if next > until_ms => {
                    clock.set_at_least(until_ms);
                    return true;
                }
                _ => {}
            }
            if self.step(clock, &mut sink).is_none() {
                return false;
            }
        }
        true
    }

    fn peek_timestamp_ms(&self) -> Option<Millis> {
        let line = self.peek_line()?;
        self.header.parse(line).map(|(ts, _)| ts.0 * 1000)
    }

    fn peek_line(&self) -> Option<&'a [u8]> {
        if self.pos >= self.buf.len() {
            return None;
        }
        let rest = &self.buf[self.pos..];
        let end = memchr(b'\n', rest).unwrap_or(rest.len());
        Some(strip_cr(&rest[..end]))
    }

    fn next_line(&mut self) -> Option<&'a [u8]> {
        if self.pos >= self.buf.len() {
            return None;
        }
        let rest = &self.buf[self.pos..];
        match memchr(b'\n', rest) {
            Some(i) => {
                self.pos += i + 1;
                Some(strip_cr(&rest[..i]))
            }
            None => {
                self.pos = self.buf.len();
                Some(strip_cr(rest))
            }
        }
    }
}

#[inline]
fn memchr(needle: u8, hay: &[u8]) -> Option<usize> {
    hay.iter().position(|&b| b == needle)
}

#[inline]
fn strip_cr(b: &[u8]) -> &[u8] {
    match b.split_last() {
        Some((&b'\r', rest)) => rest,
        _ => b,
    }
}
