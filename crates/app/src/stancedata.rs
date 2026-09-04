//! why: stance -> class lookup, one of the packs `classpool.rs` serves
//!
//! 9 real stances confirmed against eqlwiki, "Berserker" unambiguous
//! (one class). Only ever evidence for "You" -- log reports no one else's.
//! Case-insensitive against the log's own spelling; this used to scan
//! every key on every lookup instead of indexing once.
use crate::classpool::{self, ClassPool};
use std::sync::OnceLock;

static POOL: OnceLock<ClassPool> = OnceLock::new();

fn pool() -> &'static ClassPool {
    POOL.get_or_init(|| {
        ClassPool::load(
            "stance_classes.json",
            include_str!("../../../packs/stance_classes.json"),
            classpool::ci,
            &[],
        )
    })
}

/// why: empty means unknown name, not zero eligible classes
pub fn classes_for(stance: &str) -> &'static [String] {
    pool().classes_for(stance)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn berserker_is_unambiguous() {
        assert_eq!(classes_for("Berserker"), &["Berserker".to_string()]);
    }

    #[test]
    fn evasive_does_not_include_druid() {
        // why: easy mix-up, Channeler includes Druid, Evasive doesn't
        let classes = classes_for("Evasive");
        assert!(classes.contains(&"Ranger".to_string()));
        assert!(classes.contains(&"Bard".to_string()));
        assert!(!classes.contains(&"Druid".to_string()));
    }

    #[test]
    fn an_unrecognized_stance_is_unknown_not_ineligible() {
        assert!(classes_for("Not A Real Stance").is_empty());
    }

    #[test]
    fn lookup_is_case_insensitive_matching_the_log_s_own_lowercase_form() {
        assert_eq!(classes_for("evasive"), classes_for("Evasive"));
        assert_eq!(classes_for("mage hunter"), classes_for("Mage Hunter"));
    }
}
