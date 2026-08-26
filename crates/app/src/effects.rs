//! why: overlay's timed-effects tracker -- Charm/Invisibility/Hide/Sneak.
//! Every signal here is a real log line already matched by the pack, but
//! before this module existed each one either went unmapped entirely
//! (invis.fading, and every hide/sneak outcome) or was swallowed into
//! state.misc's generic bundle with no Action of its own -- see
//! ingest.rs's `flush_cast_resolutions` doc for the shape of bug that
//! kind of silent gap produces. Self-only: none of these lines name a
//! third party, so there's no attribution question to solve, unlike
//! combat's live_meter.

use crate::ingest::Ingest;
use eqlp_source::Millis;
use serde::Serialize;

/// why: `who` scopes state.charm_broken -- that pattern is generic
/// ("Your <any spell> spell has worn off of <target>"), so only a match
/// against the currently-tracked charm's own target counts as it ending;
/// unrelated buffs wearing off elsewhere in the log don't touch this.
#[derive(Debug, Clone)]
pub struct CharmStatus {
    pub who: String,
    pub active: bool,
    pub since_ms: Millis,
}

#[derive(Debug, Clone, Copy)]
pub struct InvisStatus {
    pub active: bool,
    pub fading: bool,
    pub since_ms: Millis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MomentaryOutcome {
    Success,
    Failure,
    /// why: a `hide.broken`/`hide.stopped`-shaped line -- was active, now
    /// isn't, distinct from a fresh failed attempt
    Ended,
}

impl MomentaryOutcome {
    fn label(self) -> &'static str {
        match self {
            MomentaryOutcome::Success => "success",
            MomentaryOutcome::Failure => "failure",
            MomentaryOutcome::Ended => "ended",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct MomentaryStatus {
    pub outcome: MomentaryOutcome,
    pub since_ms: Millis,
}

#[derive(Debug, Clone, Serialize)]
pub struct CharmDto {
    pub who: String,
    pub active: bool,
    pub since_ms: Millis,
}

#[derive(Debug, Clone, Serialize)]
pub struct InvisDto {
    pub active: bool,
    pub fading: bool,
    pub since_ms: Millis,
}

#[derive(Debug, Clone, Serialize)]
pub struct MomentaryDto {
    pub outcome: &'static str,
    pub since_ms: Millis,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct StatusEffectsDto {
    pub charm: Option<CharmDto>,
    pub invis: Option<InvisDto>,
    pub hide: Option<MomentaryDto>,
    pub sneak: Option<MomentaryDto>,
}

/// why: the overlay's own poll -- same shape as combat::live_meter,
/// called fresh on every parse-tick rather than pushed, no history kept
pub fn status_effects(ing: &Ingest) -> StatusEffectsDto {
    StatusEffectsDto {
        charm: ing.charm.as_ref().map(|c| CharmDto {
            who: c.who.clone(),
            active: c.active,
            since_ms: c.since_ms,
        }),
        invis: ing.invis.map(|s| InvisDto {
            active: s.active,
            fading: s.fading,
            since_ms: s.since_ms,
        }),
        hide: ing.hide.map(|s| MomentaryDto {
            outcome: s.outcome.label(),
            since_ms: s.since_ms,
        }),
        sneak: ing.sneak.map(|s| MomentaryDto {
            outcome: s.outcome.label(),
            since_ms: s.since_ms,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::backfill_lines;
    use crate::parser::build_engine;

    fn run(lines: &[&str]) -> Ingest {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let bytes: Vec<&[u8]> = lines.iter().map(|l| l.as_bytes()).collect();
        backfill_lines(&mut ing, &engine, &bytes, 1);
        ing
    }

    #[test]
    fn no_effects_yet_is_all_none_not_a_panic() {
        let ing = run(&[]);
        let dto = status_effects(&ing);
        assert!(dto.charm.is_none());
        assert!(dto.invis.is_none());
        assert!(dto.hide.is_none());
        assert!(dto.sneak.is_none());
    }

    #[test]
    fn a_charm_landing_then_breaking_reports_active_then_inactive() {
        let ing = run(&["[Tue Jul 28 15:01:00 2026] an abhorrent has been charmed."]);
        let dto = status_effects(&ing);
        let c = dto.charm.expect("a charm should be tracked");
        assert_eq!(c.who, "an abhorrent");
        assert!(c.active);

        let ing = run(&[
            "[Tue Jul 28 15:01:00 2026] an abhorrent has been charmed.",
            "[Tue Jul 28 15:01:05 2026] Your Allure spell has worn off of an abhorrent.",
        ]);
        let dto = status_effects(&ing);
        assert!(!dto.charm.expect("still tracked, now inactive").active);
    }

    /// why: real bug shape this guards against -- state.charm_broken's
    /// pattern is generic ("Your <any spell> spell has worn off of
    /// <target>"), so an unrelated buff wearing off elsewhere must not
    /// false-clear a still-active charm on a different target
    #[test]
    fn an_unrelated_spell_wearing_off_a_different_target_does_not_clear_the_charm() {
        let ing = run(&[
            "[Tue Jul 28 15:01:00 2026] an abhorrent has been charmed.",
            "[Tue Jul 28 15:01:05 2026] Your Skin like Steel spell has worn off of You.",
        ]);
        let dto = status_effects(&ing);
        assert!(
            dto.charm.expect("still tracked").active,
            "a different target's buff wearing off must not clear this charm"
        );
    }

    #[test]
    fn invis_lands_then_warns_then_ends() {
        let ing = run(&["[Tue Jul 28 15:01:00 2026] You vanish."]);
        let dto = status_effects(&ing);
        let s = dto.invis.expect("invis should be tracked");
        assert!(s.active && !s.fading);

        let ing = run(&[
            "[Tue Jul 28 15:01:00 2026] You vanish.",
            "[Tue Jul 28 15:05:00 2026] You feel yourself starting to appear.",
        ]);
        let dto = status_effects(&ing);
        let s = dto.invis.expect("invis should still be tracked");
        assert!(
            s.active && s.fading,
            "fading is still active, just about to end"
        );

        let ing = run(&[
            "[Tue Jul 28 15:01:00 2026] You vanish.",
            "[Tue Jul 28 15:05:00 2026] You feel yourself starting to appear.",
            "[Tue Jul 28 15:05:03 2026] You appear.",
        ]);
        let dto = status_effects(&ing);
        assert!(!dto.invis.expect("still tracked, now inactive").active);
    }

    #[test]
    fn hide_reports_success_then_failure_then_ended() {
        let ing = run(&["[Tue Jul 28 15:01:00 2026] You have hidden yourself from view."]);
        assert_eq!(
            status_effects(&ing).hide.expect("tracked").outcome,
            "success"
        );

        let ing = run(&["[Tue Jul 28 15:01:00 2026] You failed to hide yourself."]);
        assert_eq!(
            status_effects(&ing).hide.expect("tracked").outcome,
            "failure"
        );

        let ing = run(&[
            "[Tue Jul 28 15:01:00 2026] You have hidden yourself from view.",
            "[Tue Jul 28 15:01:05 2026] You have moved and are no longer hidden!",
        ]);
        assert_eq!(status_effects(&ing).hide.expect("tracked").outcome, "ended");
    }

    #[test]
    fn sneak_reports_success_then_failure() {
        let ing = run(&["[Tue Jul 28 15:01:00 2026] You are as quiet as a cat stalking its prey."]);
        assert_eq!(
            status_effects(&ing).sneak.expect("tracked").outcome,
            "success"
        );

        let ing =
            run(&["[Tue Jul 28 15:01:00 2026] You are as quiet as a herd of running elephants."]);
        assert_eq!(
            status_effects(&ing).sneak.expect("tracked").outcome,
            "failure"
        );
    }
}
