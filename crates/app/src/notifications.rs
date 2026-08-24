//! why: sound-notification framework, a thin second look at rule ids
//! `ingest::route` already matched -- decoupled from `ingest::Action`,
//! which decides what a line means for the parsed model, not whether
//! it's worth a sound. A rule can feed both, neither, or just one.
//!
//! Four kinds, each confirmed against the real reference log: invis
//! fading (real early-warning line, 9 occurrences), charm breaking (no
//! early warning exists, fires on the actual break line), level up, AA
//! gained -- all unambiguous single log lines.

use eqlp_core::event::Match;
use eqlp_core::{field, Engine};
use eqlp_source::Millis;
use serde::Serialize;

/// why: stable ids, also settings/frontend lookup keys -- never rename
pub const INVIS_FADING: &str = "invis_fading";
pub const CHARM_BROKEN: &str = "charm_broken";
pub const LEVEL_UP: &str = "level_up";
pub const AA_GAINED: &str = "aa_gained";

/// why: single place the full kind list is defined, add a new kind once
pub const ALL_KINDS: &[&str] = &[INVIS_FADING, CHARM_BROKEN, LEVEL_UP, AA_GAINED];

pub fn kind_label(kind: &str) -> &'static str {
    match kind {
        INVIS_FADING => "Invisibility fading",
        CHARM_BROKEN => "Charm breaking",
        LEVEL_UP => "Level up",
        AA_GAINED => "AA gained",
        _ => "Unknown",
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct NotificationEvent {
    /// why: one of the `*_KIND` constants above
    pub kind: String,
    /// why: human-readable, ready to show in a toast as-is
    pub message: String,
    pub ts_ms: Millis,
}

/// why: None for the overwhelming majority of rule ids; no separate re-scan
pub fn notification_for(
    engine: &Engine,
    rule_id: &str,
    m: &Match,
    line: &[u8],
    ts_ms: Millis,
) -> Option<NotificationEvent> {
    let str_field = |name: &str| -> Option<String> {
        match field::field(engine, m, line, name) {
            field::Value::Str(s) => Some(String::from_utf8_lossy(s).into_owned()),
            _ => None,
        }
    };
    let u64_field = |name: &str| -> Option<u64> {
        match field::field(engine, m, line, name) {
            field::Value::U64(n) => Some(n),
            _ => None,
        }
    };

    let (kind, message) = match rule_id {
        "invis.fading" => (INVIS_FADING, "Invisibility fading".to_string()),
        "state.charm_broken" => (CHARM_BROKEN, format!("Charm broke: {}", str_field("who")?)),
        "level.up" => (LEVEL_UP, format!("Level {}!", u64_field("level")?)),
        "aa.gained" => (AA_GAINED, format!("Gained AA: {}", str_field("name")?)),
        "aa.improved" => (
            AA_GAINED,
            format!(
                "Improved AA: {} (rank {})",
                str_field("name")?,
                u64_field("rank")?
            ),
        ),
        _ => return None,
    };
    Some(NotificationEvent {
        kind: kind.to_string(),
        message,
        ts_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::build_engine;

    /// why: real trigger lines through the real parser, confirms rule + mapping agree
    #[test]
    fn real_trigger_lines_produce_the_right_notification() {
        let engine = build_engine().expect("pack builds");
        let mut matcher = engine.matcher();
        let cases: &[(&[u8], &str, &str)] = &[
            (b"[Fri Jul 31 00:20:03 2026] You feel yourself starting to appear.", INVIS_FADING, "Invisibility fading"),
            (
                b"[Tue Jul 28 15:02:15 2026] Your Allure spell has worn off of an abhorrent.",
                CHARM_BROKEN,
                "Charm broke: an abhorrent",
            ),
            (b"[Tue Jul 28 15:02:15 2026] You have gained a level! Welcome to level 2!", LEVEL_UP, "Level 2!"),
            (
                b"[Fri Aug 07 00:25:51 2026] You have gained the ability \"Unbound Drain\" at a cost of 0 ability points.",
                AA_GAINED,
                "Gained AA: Unbound Drain",
            ),
            (
                b"[Mon Aug 10 09:00:00 2026] You have improved Spell Casting Deftness 2 at a cost of 4 ability points.",
                AA_GAINED,
                "Improved AA: Spell Casting Deftness (rank 2)",
            ),
        ];
        for (line, expected_kind, expected_message) in cases {
            let outcome = matcher.classify(line);
            let eqlp_core::Outcome::Matched(m) = outcome else {
                panic!(
                    "line should have matched a rule: {}",
                    String::from_utf8_lossy(line)
                );
            };
            let rule = engine.rule(m.rule);
            let notif =
                notification_for(&engine, rule.id.as_str(), &m, line, 0).unwrap_or_else(|| {
                    panic!(
                        "{} should have produced a notification",
                        String::from_utf8_lossy(line)
                    )
                });
            assert_eq!(notif.kind, *expected_kind);
            assert_eq!(notif.message, *expected_message);
        }
    }

    /// why: an ordinary combat line, the common case, must produce nothing
    #[test]
    fn an_unrelated_matched_line_produces_nothing() {
        let engine = build_engine().expect("pack builds");
        let line: &[u8] =
            b"[Tue Jul 28 15:02:15 2026] Bouncer Krik slashes Beba for 59 points of damage.";
        let mut matcher = engine.matcher();
        let outcome = matcher.classify(line);
        let eqlp_core::Outcome::Matched(m) = outcome else {
            panic!("should match melee.hit")
        };
        let rule = engine.rule(m.rule);
        assert!(notification_for(&engine, rule.id.as_str(), &m, line, 0).is_none());
    }
}
