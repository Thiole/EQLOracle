//! Sound-notification framework: which real, already-classified log lines
//! are worth alerting the player to, live, with a sound -- not a new
//! parsing concern of its own, just a thin second look at a handful of
//! rule ids `ingest::route` already has a `Match` for. Deliberately
//! decoupled from `ingest::extract_action`/`Action`: that pipeline decides
//! what a line *means* for the parsed model (encounters, timeline, XP,
//! ...), this decides whether it's the kind of thing someone alt-tabbed
//! away from the game would want a sound for. A rule can matter to both,
//! neither, or just one -- `state.charm_broken` already feeds `Action::
//! Recovered` for Timeline purposes *and* produces a notification here;
//! `melee.hit` feeds combat scoring and produces no notification at all.
//!
//! Four kinds to start (`packs/eql.toml`'s own `invis.fading`/`state.
//! charm_broken`/`level.up`/`aa.gained`/`aa.improved` rules), each
//! confirmed against the real reference log before being wired up:
//!   - Invisibility fading: "You feel yourself starting to appear." is a
//!     real, distinct early-warning line (9 real occurrences) -- invis
//!     genuinely gives advance notice before it drops.
//!   - Charm breaking: no equivalent early-warning line exists for charm
//!     (checked; there's no "about to break free" text anywhere in the
//!     reference log) -- "dropping" here means the actual break event
//!     (`Your <spell> spell has worn off of <target>.`), not a heads-up.
//!   - Level up / AA gained: both already have a real, unambiguous log
//!     line each with nothing to disambiguate.

use eqlp_core::event::Match;
use eqlp_core::{field, Engine};
use eqlp_source::Millis;
use serde::Serialize;

/// Stable identifiers -- also the notification settings' own keys (see
/// `crate::settings::NotificationSettings`) and the frontend's lookup key
/// for which sound to play. Never renamed once shipped: a saved settings
/// file keys its `enabled`/custom-sound map by these exact strings.
pub const INVIS_FADING: &str = "invis_fading";
pub const CHARM_BROKEN: &str = "charm_broken";
pub const LEVEL_UP: &str = "level_up";
pub const AA_GAINED: &str = "aa_gained";

/// Every kind this framework knows about, in the order the Settings
/// module lists them -- the single place that list is defined, so a
/// fifth kind is added here once, not once per consumer.
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
    /// One of the `*_KIND` constants above.
    pub kind: String,
    /// Human-readable, ready to show in a toast as-is.
    pub message: String,
    pub ts_ms: Millis,
}

/// `None` for every rule id this framework doesn't care about -- the
/// overwhelmingly common case (thousands of matched lines a minute during
/// combat, a handful of notification-worthy ones a session). Only ever
/// called for a `Match` (a matched line already has `m`/`line` available
/// for free -- no separate text re-scan).
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

    /// Every one of the 4 real trigger lines from the actual reference
    /// log, run end to end through the real parser -- confirms both the
    /// rule (`packs/eql.toml`) and this mapping agree on what each one
    /// means. `backfill_lines` doesn't collect notifications on its own
    /// (that only happens on the live path -- see `ingest::route`'s own
    /// doc), so this drives `notification_for` directly against real
    /// `Match`es the same way `route` would.
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

    /// An ordinary combat line -- the overwhelmingly common case -- must
    /// produce no notification at all.
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
