//! Bytes to lines, with a carry buffer for partial writes while tailing.
//!
//! Design notes: `docs/design/parsing.md`

use memchr::memchr_iter;

pub const DEFAULT_MAX_LINE: usize = 64 * 1024;

pub struct Framer {
    carry: Vec<u8>,
    max_line: usize,
    /// Set after emitting an over-long line; discard bytes until the next `\n`.
    resyncing: bool,
    pub truncated: u64,
    pub lines_out: u64,
}

impl Default for Framer {
    fn default() -> Self {
        Framer::new(DEFAULT_MAX_LINE)
    }
}

impl Framer {
    pub fn new(max_line: usize) -> Self {
        Framer {
            carry: Vec::with_capacity(256),
            max_line: max_line.max(64),
            resyncing: false,
            truncated: 0,
            lines_out: 0,
        }
    }

    /// Feed a chunk. `f` is called once per complete line, with `\r\n` and `\n`
    /// already stripped. The slice is only valid for the duration of the call.
    pub fn push(&mut self, mut chunk: &[u8], mut f: impl FnMut(&[u8])) {
        if self.resyncing {
            match memchr::memchr(b'\n', chunk) {
                Some(i) => {
                    chunk = &chunk[i + 1..];
                    self.resyncing = false;
                }
                None => return,
            }
        }

        let mut start = 0usize;
        for nl in memchr_iter(b'\n', chunk) {
            let raw = &chunk[start..nl];
            if self.carry.is_empty() {
                self.lines_out += 1;
                f(strip_cr(raw));
            } else {
                self.carry.extend_from_slice(raw);
                self.lines_out += 1;
                let carry = std::mem::take(&mut self.carry);
                f(strip_cr(&carry));
                let mut carry = carry;
                carry.clear();
                self.carry = carry; // reuse the allocation
            }
            start = nl + 1;
        }

        let tail = &chunk[start..];
        if !tail.is_empty() {
            if self.carry.len() + tail.len() > self.max_line {
                let room = self.max_line.saturating_sub(self.carry.len());
                self.carry.extend_from_slice(&tail[..room.min(tail.len())]);
                let carry = std::mem::take(&mut self.carry);
                self.truncated += 1;
                self.lines_out += 1;
                f(strip_cr(&carry));
                let mut carry = carry;
                carry.clear();
                self.carry = carry;
                self.resyncing = true;
            } else {
                self.carry.extend_from_slice(tail);
            }
        }
    }

    /// Emit whatever is buffered. Call at EOF of a batch parse; do *not* call
    /// while tailing a live file — the partial line is not partial by mistake,
    /// the game just has not finished writing it.
    pub fn flush(&mut self, mut f: impl FnMut(&[u8])) {
        if !self.carry.is_empty() {
            let carry = std::mem::take(&mut self.carry);
            self.lines_out += 1;
            f(strip_cr(&carry));
        }
    }

    pub fn pending(&self) -> usize {
        self.carry.len()
    }
}

#[inline]
fn strip_cr(b: &[u8]) -> &[u8] {
    if let Some((&last, rest)) = b.split_last() {
        if last == b'\r' {
            return rest;
        }
    }
    b
}

/// Non-streaming convenience for whole-buffer parses.
pub fn lines(buf: &[u8]) -> impl Iterator<Item = &[u8]> {
    let mut start = 0usize;
    let mut it = memchr_iter(b'\n', buf);
    std::iter::from_fn(move || match it.next() {
        Some(nl) => {
            let s = &buf[start..nl];
            start = nl + 1;
            Some(strip_cr(s))
        }
        None if start < buf.len() => {
            let s = &buf[start..];
            start = buf.len();
            Some(strip_cr(s))
        }
        None => None,
    })
}
