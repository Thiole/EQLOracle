//! Optional stage: capture spans to typed values.
//!
//! Design notes: `docs/design/parsing.md`

use crate::engine::Engine;
use crate::event::{Match, RuleIdx};
use crate::rule::FieldType;

#[derive(Debug, Clone, PartialEq)]
pub enum Value<'a> {
    Str(&'a [u8]),
    U64(u64),
    I64(i64),
    F64(f64),
    Bool(bool),
    Missing,
}

/// Resolve a `from` spec (group name or 1-based index) to a group index.
fn group_index(eng: &Engine, rule: RuleIdx, from: &str) -> Option<usize> {
    if let Ok(n) = from.parse::<usize>() {
        return if n >= 1 { Some(n - 1) } else { None };
    }
    let names = &eng.rule(rule).cap_names;
    names
        .iter()
        .position(|n| n.as_deref() == Some(from))
        .map(|i| i.saturating_sub(1))
}

pub fn field<'a>(eng: &Engine, m: &Match, line: &'a [u8], name: &str) -> Value<'a> {
    let def = match eng.rule(m.rule).def.fields.get(name) {
        Some(d) => d,
        None => return Value::Missing,
    };
    let gi = match group_index(eng, m.rule, &def.from) {
        Some(g) => g,
        None => return Value::Missing,
    };
    let raw = match m.cap(line, gi) {
        Some(r) => r,
        None => return Value::Missing,
    };
    coerce(raw, def.ty)
}

pub fn coerce(raw: &[u8], ty: FieldType) -> Value<'_> {
    match ty {
        FieldType::Str => Value::Str(raw),
        FieldType::U64 => parse_u64(raw).map(Value::U64).unwrap_or(Value::Missing),
        FieldType::I64 => {
            let (neg, rest) = match raw.first() {
                Some(b'-') => (true, &raw[1..]),
                Some(b'+') => (false, &raw[1..]),
                _ => (false, raw),
            };
            parse_u64(rest)
                .and_then(|v| i64::try_from(v).ok())
                .map(|v| Value::I64(if neg { -v } else { v }))
                .unwrap_or(Value::Missing)
        }
        FieldType::F64 => std::str::from_utf8(raw)
            .ok()
            .and_then(|s| s.parse::<f64>().ok())
            .map(Value::F64)
            .unwrap_or(Value::Missing),
        FieldType::Bool => match raw {
            b"true" | b"1" | b"yes" => Value::Bool(true),
            b"false" | b"0" | b"no" => Value::Bool(false),
            _ => Value::Missing,
        },
    }
}

/// why: digit-only, comma-tolerant, overflow-checked, no UTF-8 requirement
#[inline]
pub fn parse_u64(raw: &[u8]) -> Option<u64> {
    let mut v: u64 = 0;
    let mut any = false;
    for &c in raw {
        if c == b',' || c == b'_' {
            continue;
        }
        if !c.is_ascii_digit() {
            return None;
        }
        v = v.checked_mul(10)?.checked_add((c - b'0') as u64)?;
        any = true;
    }
    if any {
        Some(v)
    } else {
        None
    }
}
