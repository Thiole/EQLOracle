//! why: log mob names don't always match the wiki bestiary title
//!
//! Same gap `zone.rs`'s `ZONE_ALIASES` closes for zones. Confirmed case:
//! log says "Innoruuk, the Prince of Hate", `packs/npcs.json` has just
//! "Innoruuk" -- the "(God)" disambiguator lives only in the id slug.

/// why: log name -> wiki `Npc::name`, add only on a confirmed mismatch
const MOB_ALIASES: &[(&str, &str)] = &[
    ("Innoruuk, the Prince of Hate", "Innoruuk"),
    ("Cazic-Thule", "Cazic Thule"),
];

/// why: the other direction -- what the log calls a wiki mob; the wiki
/// name itself when the two agree. The raid tab and the mob page both
/// look kills up under this.
pub fn log_name(wiki_name: &str) -> &str {
    MOB_ALIASES
        .iter()
        .find(|&&(_, w)| w.eq_ignore_ascii_case(wiki_name))
        .map(|&(log, _)| log)
        .unwrap_or(wiki_name)
}

/// why: the one function anything should call to match log mob to wiki mob
pub fn mob_matches(raw: &str, wiki_name: &str) -> bool {
    if raw.eq_ignore_ascii_case(wiki_name) {
        return true;
    }
    MOB_ALIASES
        .iter()
        .any(|(k, v)| k.eq_ignore_ascii_case(raw) && v.eq_ignore_ascii_case(wiki_name))
}

/// why: exposes the table to the frontend, avoids a second driftable copy
pub fn all() -> &'static [(&'static str, &'static str)] {
    MOB_ALIASES
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn innoruuk_matches_the_wiki_s_shorter_name() {
        assert!(mob_matches("Innoruuk, the Prince of Hate", "Innoruuk"));
    }

    #[test]
    fn an_ordinary_exact_match_still_works() {
        assert!(mob_matches("a gnoll", "a gnoll"));
        assert!(mob_matches("A Gnoll", "a gnoll"));
    }

    #[test]
    fn two_different_mobs_never_match() {
        assert!(!mob_matches("Innoruuk, the Prince of Hate", "Cazic-Thule"));
    }
}
