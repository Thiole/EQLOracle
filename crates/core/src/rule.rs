//! why: rule pack schema/layering/errors -- data only, kind is opaque
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
    /// why: stable identity -- renaming is breaking, reuse is a merge
    pub id: String,

    /// why: anchor with `^` unless you mean not to
    #[serde(default)]
    pub pattern: String,

    /// why: Aho-Corasick prefilter, reduces regexes to zero-or-one hit
    #[serde(default)]
    pub anchors: Vec<String>,

    /// why: literal veto -- no lookaround, so this stands in for it
    #[serde(default)]
    pub excludes: Vec<String>,

    /// why: higher wins on conflict, ties break on declaration order
    #[serde(default)]
    pub priority: i32,

    /// why: opaque to this crate, consumers switch on it
    #[serde(default)]
    pub kind: String,

    /// why: optional typed reading of capture groups
    #[serde(default)]
    pub fields: BTreeMap<String, FieldDef>,

    #[serde(default = "default_true")]
    pub enabled: bool,

    /// why: executable -- `eqlp lint` fails if another rule wins instead
    #[serde(default)]
    pub examples: Vec<String>,

    /// why: pins near-miss regressions
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

/// why: layered packs, stable order, later-pack rules appended
#[derive(Debug, Clone, Default)]
pub struct ResolvedPack {
    pub rules: Vec<RuleDef>,
    /// why: kept so tooling can explain "why isn't my rule firing"
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
        Ok(ResolvedPack {
            rules,
            disabled,
            sources,
            header,
        })
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
