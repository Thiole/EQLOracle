//! Splitting a line into `(timestamp, body)`. Pluggable: the log format is the
//! one thing the engine must know, so it is a trait.
//!
//! Design notes: `docs/design/parsing.md`

use crate::event::LocalTs;

pub trait HeaderParser: Send + Sync {
    /// `Some((ts, body_offset))` on success. Must not panic on any input.
    fn parse(&self, line: &[u8]) -> Option<(LocalTs, usize)>;
    fn name(&self) -> &'static str;
}

/// `[Wed Aug 06 21:14:33 2025] body...`
///
/// why: fixed-offset, no chrono, accepts zero- or space-padded day
#[derive(Default, Clone, Copy, Debug)]
pub struct BracketCtime;

const HEADER_LEN: usize = 26; // '[' + 24 + ']'

#[inline]
fn month_idx(b: &[u8]) -> Option<u32> {
    Some(match b {
        b"Jan" => 1,
        b"Feb" => 2,
        b"Mar" => 3,
        b"Apr" => 4,
        b"May" => 5,
        b"Jun" => 6,
        b"Jul" => 7,
        b"Aug" => 8,
        b"Sep" => 9,
        b"Oct" => 10,
        b"Nov" => 11,
        b"Dec" => 12,
        _ => return None,
    })
}

#[inline]
fn d2(b: &[u8]) -> Option<u32> {
    let (hi, lo) = (b[0], b[1]);
    let h = if hi == b' ' {
        0
    } else if hi.is_ascii_digit() {
        (hi - b'0') as u32
    } else {
        return None;
    };
    if !lo.is_ascii_digit() {
        return None;
    }
    Some(h * 10 + (lo - b'0') as u32)
}

#[inline]
fn d4(b: &[u8]) -> Option<i64> {
    let mut v = 0i64;
    for &c in b {
        if !c.is_ascii_digit() {
            return None;
        }
        v = v * 10 + (c - b'0') as i64;
    }
    Some(v)
}

/// why: Hinnant's civil-from-days, no lookup tables, no leap-year branches
#[inline]
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let mp = ((m + 9) % 12) as i64; // Mar = 0
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

impl HeaderParser for BracketCtime {
    #[inline]
    fn parse(&self, line: &[u8]) -> Option<(LocalTs, usize)> {
        if line.len() < HEADER_LEN || line[0] != b'[' || line[25] != b']' {
            return None;
        }
        if line[4] != b' ' || line[8] != b' ' || line[11] != b' ' || line[20] != b' ' {
            return None;
        }
        if line[14] != b':' || line[17] != b':' {
            return None;
        }
        let mo = month_idx(&line[5..8])?;
        let d = d2(&line[9..11])?;
        let hh = d2(&line[12..14])?;
        let mi = d2(&line[15..17])?;
        let ss = d2(&line[18..20])?;
        let y = d4(&line[21..25])?;

        // why: leap second (60) tolerated, only reject impossible clocks
        if d == 0 || d > 31 || hh > 23 || mi > 59 || ss > 60 {
            return None;
        }

        let secs = days_from_civil(y, mo, d) * 86_400 + (hh * 3600 + mi * 60 + ss) as i64;

        let mut off = HEADER_LEN;
        while off < line.len() && line[off] == b' ' {
            off += 1;
        }
        Some((LocalTs(secs), off))
    }

    fn name(&self) -> &'static str {
        "bracket-ctime"
    }
}

/// why: escape hatch for headerless sources -- whole line as body, ts=0
/// why: one real header shape on this game's logs; the pack names it
/// (`header = "bracket-ctime"`) and nothing else is selectable
pub fn by_name(name: &str) -> Option<Box<dyn HeaderParser>> {
    (name == "bracket-ctime").then(|| Box::new(BracketCtime) as Box<dyn HeaderParser>)
}
