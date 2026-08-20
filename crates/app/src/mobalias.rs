//! A mob's name as the log states it doesn't always match the wiki's own
//! bestiary page title -- same gap `zone.rs`'s `ZONE_ALIASES` closes for
//! zones, for the same reason (two independent scrapes/sources, not a
//! formatting difference `eq_ignore_ascii_case` alone can bridge).
//!
//! Confirmed real case: the log calls him "Innoruuk, the Prince of Hate"
//! (`packs/eql.toml`'s own `melee.hit` examples), but `packs/npcs.json`'s
//! entry (id `Innoruuk_(God)`) has `name: "Innoruuk"` -- the "(God)"
//! disambiguator lives only in the id/URL slug, and the log's own longer
//! title doesn't appear on the wiki page at all.

/// Log name -> wiki `Npc::name`. Keep short; add an entry only once a real
/// mismatch is confirmed against `packs/npcs.json`, the same standard
/// `ZONE_ALIASES` holds itself to.
const MOB_ALIASES: &[(&str, &str)] = &[("Innoruuk, the Prince of Hate", "Innoruuk")];

/// Whether `raw` (a mob name as `Store::name` / an encounter's own target
/// holds it) and `wiki_name` (`npcdata::Npc::name`) refer to the same
/// mob. The one function anything cross-referencing a log mob name
/// against the bestiary should call.
pub fn mob_matches(raw: &str, wiki_name: &str) -> bool {
    if raw.eq_ignore_ascii_case(wiki_name) {
        return true;
    }
    MOB_ALIASES
        .iter()
        .any(|(k, v)| k.eq_ignore_ascii_case(raw) && v.eq_ignore_ascii_case(wiki_name))
}

/// Every alias pair, for exposing to the frontend (`get_name_aliases`) so
/// Game Data's own cross-links can resolve the same mismatches this file
/// already knows about, without a second, driftable copy of the table.
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
