//! Rule pack schema, layering, and errors. Rules are data; this crate does not
//! know what any `kind` means.
//!
//! Design notes: `docs/design/parsing.md`

use serde::Deserialize;
use std::collections::BTreeMap;

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pack {
    pub name: String,
    #[serde(default)]
    pub version: u32,
    /// Header parser name; see `header::by_name`. Defaults to bracket-ctime.
    #[serde(default)]
    pub header: Option<String>,
    #[serde(default, rename = "rule")]
    pub rules: Vec<RuleDef>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleDef {
    /// Stable identity. Overrides and telemetry key off this, so renaming a
    /// rule is a breaking change and reusing an id is a merge.
    pub id: String,

    /// Byte regex applied to the message body. Anchor it with `^` unless you
    /// mean not to.
    #[serde(default)]
    pub pattern: String,

    /// Literal substrings that must all be present for the rule to even be
    /// considered. This is the whole performance story: one Aho-Corasick pass
    /// over the line reduces hundreds of regexes to zero or one.
    ///
    /// Empty is legal and correct, just slow — `lint` will warn.
    #[serde(default)]
    pub anchors: Vec<String>,

    /// Literal substrings that must be ABSENT for the rule to be considered.
    ///
    /// Rust's regex engine has no lookaround (deliberately — it is what buys
    /// the linear-time guarantee), so "match X but not when Y is present" has
    /// no clean regex spelling. Expressing it as a literal veto is clearer than
    /// a contorted pattern, and it is free: the exclusion literals ride along
    /// in the same Aho-Corasick pass that finds the anchors.
    #[serde(default)]
    pub excludes: Vec<String>,

    /// Higher wins when several rules match. Ties break on declaration order,
    /// so matching is fully deterministic and diffable.
    #[serde(default)]
    pub priority: i32,

    /// Opaque to this crate. Consumers switch on it.
    #[serde(default)]
    pub kind: String,

    /// Optional typed reading of capture groups, for consumers that want it.
    #[serde(default)]
    pub fields: BTreeMap<String, FieldDef>,

    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Lines this rule must match. These are executable: `eqlp lint` runs every
    /// example through the *whole* engine and fails if another rule wins, which
    /// catches shadowing the moment it is introduced.
    #[serde(default)]
    pub examples: Vec<String>,

    /// Lines this rule must not match. Where near-miss regressions get pinned.
    #[serde(default)]
    pub counterexamples: Vec<String>,

    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldDef {
    /// Capture group name or 1-based index.
    pub from: String,
    #[serde(default, rename = "as")]
    pub ty: FieldType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum FieldType {
    #[default]
    Str,
    U64,
    I64,
    F64,
    Bool,
}

impl Pack {
    pub fn from_toml(s: &str) -> Result<Pack, PackError> {
        toml::from_str(s).map_err(|e| PackError::Toml(e.to_string()))
    }
}

/// The result of layering N packs. Order is stable: base declaration order,
/// with rules introduced by later packs appended.
#[derive(Debug, Clone, Default)]
pub struct ResolvedPack {
    pub rules: Vec<RuleDef>,
    /// Rules disabled by a later layer, kept so tooling can explain "why is my
    /// rule not firing".
    pub disabled: Vec<RuleDef>,
    pub sources: Vec<String>,
    pub header: String,
}

impl ResolvedPack {
    pub fn layer(packs: Vec<Pack>) -> Result<ResolvedPack, PackError> {
        let mut order: Vec<String> = Vec::new();
        let mut by_id: BTreeMap<String, RuleDef> = BTreeMap::new();
        let mut sources = Vec::new();
        let mut header = "bracket-ctime".to_string();

        for p in packs {
            sources.push(format!("{}@{}", p.name, p.version));
            if let Some(h) = p.header {
                header = h;
            }
            let mut seen_here: std::collections::HashSet<String> = std::collections::HashSet::new();
            for r in p.rules {
                if !seen_here.insert(r.id.clone()) {
                    return Err(PackError::DuplicateId {
                        pack: p.name.clone(),
                        id: r.id,
                    });
                }
                if !by_id.contains_key(&r.id) {
                    order.push(r.id.clone());
                }
                by_id.insert(r.id.clone(), r);
            }
        }

        let mut rules = Vec::new();
        let mut disabled = Vec::new();
        for id in order {
            if let Some(r) = by_id.remove(&id) {
                if r.enabled {
                    rules.push(r)
                } else {
                    disabled.push(r)
                }
            }
        }
        Ok(ResolvedPack { rules, disabled, sources, header })
    }
}

#[derive(Debug, Clone)]
pub enum PackError {
    Toml(String),
    DuplicateId { pack: String, id: String },
    BadRegex { id: String, msg: String },
    TooManyCaps { id: String, n: usize, max: usize },
    AnchorNotLiteral { id: String, anchor: String },
    EmptyPattern { id: String },
}

impl std::fmt::Display for PackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PackError::Toml(m) => write!(f, "pack parse error: {m}"),
            PackError::DuplicateId { pack, id } => {
                write!(f, "pack '{pack}' declares rule id '{id}' twice")
            }
            PackError::BadRegex { id, msg } => write!(f, "rule '{id}': bad regex: {msg}"),
            PackError::TooManyCaps { id, n, max } => {
                write!(f, "rule '{id}': {n} capture groups, max is {max}")
            }
            PackError::AnchorNotLiteral { id, anchor } => {
                write!(f, "rule '{id}': anchor '{anchor}' is empty")
            }
            PackError::EmptyPattern { id } => write!(f, "rule '{id}': empty pattern"),
        }
    }
}

impl std::error::Error for PackError {}
