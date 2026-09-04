//! why: one name -> classes lookup, shared by every pack that has one
//! (spells, stances, skills, invocations). Each of those was its own
//! copy of OnceLock + `include_str!` + `classes_for`, and the copies had
//! drifted into three DIFFERENT matching rules: exact keys for spells
//! and skills, a linear `eq_ignore_ascii_case` scan per lookup for
//! stances, and a normalized index for invocations. One type, one index,
//! and each pack states which folding it wants.
//!
//! Folding is per pack on purpose, not global: "Ice Strike" (Shaman) and
//! "Icestrike" (Wizard) are different spells, so the whitespace-stripping
//! the invocation log text needs would merge two real spells.

use std::collections::HashMap;

pub struct ClassPool {
    index: HashMap<String, Vec<String>>,
    fold: fn(&str) -> String,
}

/// why: log text and wiki keys differ only in case for most packs
pub fn ci(s: &str) -> String {
    s.to_lowercase()
}

/// why: the client prints invocations with its own spacing
pub fn tight(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_lowercase()
}

impl ClassPool {
    /// why: malformed embedded data is a build bug, fail loud. `aliases`
    /// maps a log spelling onto the pack's own key ("empowering" is the
    /// wiki's "Empower").
    pub fn load(
        name: &'static str,
        json: &'static str,
        fold: fn(&str) -> String,
        aliases: &[(&str, &str)],
    ) -> Self {
        let raw: HashMap<String, Vec<String>> = serde_json::from_str(json)
            .unwrap_or_else(|e| panic!("packs/{name} failed to parse: {e}"));
        let mut index: HashMap<String, Vec<String>> =
            raw.iter().map(|(k, v)| (fold(k), v.clone())).collect();
        for (spoken, key) in aliases {
            if let Some(v) = raw.get(*key) {
                index.insert(fold(spoken), v.clone());
            }
        }
        ClassPool { index, fold }
    }

    /// why: empty means unknown name, not zero eligible classes
    pub fn classes_for(&self, name: &str) -> &[String] {
        self.index
            .get(&(self.fold)(name))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// why: the real collision that keeps folding per-pack -- one global
    /// whitespace-stripping rule would merge a Shaman spell into a Wizard one
    #[test]
    fn case_folding_keeps_two_real_spells_apart() {
        let p = ClassPool::load(
            "t.json",
            r#"{"Ice Strike":["Shaman"],"Icestrike":["Wizard"]}"#,
            ci,
            &[],
        );
        assert_eq!(p.classes_for("ice strike"), &["Shaman".to_string()]);
        assert_eq!(p.classes_for("Icestrike"), &["Wizard".to_string()]);
        assert!(p.classes_for("Not A Spell").is_empty());
    }

    #[test]
    fn an_alias_resolves_to_its_packs_own_key() {
        let p = ClassPool::load(
            "t.json",
            r#"{"Empower":["Wizard"]}"#,
            tight,
            &[("empowering", "Empower")],
        );
        assert_eq!(p.classes_for("empowering"), &["Wizard".to_string()]);
        assert_eq!(p.classes_for("Empower"), &["Wizard".to_string()]);
    }
}
