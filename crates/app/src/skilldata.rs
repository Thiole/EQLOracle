//! why: skill -> class lookup, one of the packs `classpool.rs` serves
//!
//! Only skills purely class-gated count as evidence. Tracking included
//! (Bard/Druid/Ranger only). Forage deliberately excluded -- Iksar/Wood
//! Elf get it from race regardless of class, would false-positive.
//! Single-class skills verified on the wiki's own class pages
//! (2026-09-03); multi-class pools (Kick, Bash, Sneak ...) stay out
//! until every class page can be checked -- 11 have no skill section,
//! and an incomplete pool would falsely eliminate a class.
use crate::classpool::{self, ClassPool};
use std::sync::OnceLock;

static POOL: OnceLock<ClassPool> = OnceLock::new();

fn pool() -> &'static ClassPool {
    POOL.get_or_init(|| {
        ClassPool::load(
            "skill_classes.json",
            include_str!("../../../packs/skill_classes.json"),
            classpool::ci,
            &[],
        )
    })
}

/// why: empty means unknown name, not zero eligible classes
pub fn classes_for(skill: &str) -> &'static [String] {
    pool().classes_for(skill)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracking_is_bard_druid_ranger() {
        let classes = classes_for("Tracking");
        assert_eq!(classes.len(), 3);
        for c in ["Bard", "Druid", "Ranger"] {
            assert!(classes.contains(&c.to_string()), "{classes:?} missing {c}");
        }
    }

    #[test]
    fn forage_is_deliberately_not_here() {
        assert!(classes_for("Forage").is_empty());
    }

    /// why: verified on the class pages -- Rogue's Combat Skills lists
    /// Backstab and no other page does; Monk's lists Flying Kick likewise
    #[test]
    fn verified_single_class_skills_map_to_their_one_class() {
        assert_eq!(classes_for("Backstab"), &["Rogue".to_string()]);
        assert_eq!(classes_for("Flying Kick"), &["Monk".to_string()]);
        assert_eq!(classes_for("Frenzy"), &["Berserker".to_string()]);
        assert!(classes_for("Kick").is_empty(), "pools stay out");
    }

    #[test]
    fn an_unrecognized_skill_is_unknown_not_ineligible() {
        assert!(classes_for("Not A Real Skill").is_empty());
    }
}
