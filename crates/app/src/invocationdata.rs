//! why: invocation -> class lookup, one of the packs `classpool.rs` serves
//!
//! The log prints its own spelling, so this pack folds whitespace away
//! as well as case, and carries the one real alias: the client says
//! "empowering" where the wiki page is "Empower".
use crate::classpool::{self, ClassPool};
use std::sync::OnceLock;

static POOL: OnceLock<ClassPool> = OnceLock::new();

fn pool() -> &'static ClassPool {
    POOL.get_or_init(|| {
        ClassPool::load(
            "invocation_classes.json",
            include_str!("../../../packs/invocation_classes.json"),
            classpool::tight,
            &[("empowering", "Empower")],
        )
    })
}

/// why: empty means unknown name, not zero eligible classes
pub fn classes_for(invocation: &str) -> &'static [String] {
    pool().classes_for(invocation)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inviolable_is_bard_or_wizard() {
        let classes = classes_for("inviolable");
        assert_eq!(classes.len(), 2);
        assert!(classes.contains(&"Bard".to_string()));
        assert!(classes.contains(&"Wizard".to_string()));
    }

    #[test]
    fn real_log_spellings_resolve_despite_not_matching_the_wiki_s_own_casing() {
        assert_eq!(classes_for("overchannel"), classes_for("Over Channel"));
        assert_eq!(classes_for("spellblade"), classes_for("Spellblade"));
        assert_eq!(classes_for("empowering"), classes_for("Empower"));
        assert!(!classes_for("overchannel").is_empty());
    }

    #[test]
    fn an_unrecognized_invocation_is_unknown_not_ineligible() {
        assert!(classes_for("Not A Real Invocation").is_empty());
    }
}
