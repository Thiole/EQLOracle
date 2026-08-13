//! Log parsing: framing, header extraction, rule-driven classification,
//! coverage accounting. No I/O, no UI.
//!
//! Design notes: `docs/design/parsing.md`

pub mod coverage;
pub mod engine;
pub mod event;
pub mod field;
pub mod frame;
pub mod header;
pub mod rule;
pub mod shape;

pub use coverage::Coverage;
pub use engine::{Engine, Matcher};
pub use event::{LocalTs, Match, Outcome, RuleIdx, Span};
pub use rule::{Pack, PackError, ResolvedPack};
pub use shape::ShapeMode;

/// Build an engine from a list of TOML pack sources, layered left to right.
pub fn engine_from_toml(sources: &[&str]) -> Result<Engine, PackError> {
    let packs = sources
        .iter()
        .map(|s| Pack::from_toml(s))
        .collect::<Result<Vec<_>, _>>()?;
    let resolved = ResolvedPack::layer(packs)?;
    Engine::build(&resolved)
}

/// Parse a whole buffer. Returns coverage; `f` sees every outcome in order.
pub fn parse_buf(
    eng: &Engine,
    buf: &[u8],
    mode: ShapeMode,
    mut f: impl FnMut(&[u8], &Outcome),
) -> Coverage {
    let mut m = eng.matcher();
    let mut cov = Coverage::new(eng.rules().len(), mode);
    for line in frame::lines(buf) {
        let out = m.classify(line);
        cov.record(line, &out);
        f(line, &out);
    }
    cov
}
