//! why: overlay's timed-effects tracker -- Charm/Invisibility/Hide/Sneak/
//! CC (Stun/Root/Fear). Every signal here is a real log line already
//! matched by the pack, but before this module existed each one either
//! went unmapped entirely (invis.fading, and every hide/sneak outcome)
//! or was swallowed into state.misc's generic bundle with no Action of
//! its own -- see ingest.rs's `flush_cast_resolutions` doc for the shape
//! of bug that kind of silent gap produces. Self-only: none of these
//! lines name a third party, so there's no attribution question to
//! solve, unlike combat's live_meter.
//!
//! CC (Stun/Root/Fear) reuses MomentaryStatus exactly like Hide/Sneak
//! do -- Success = landed, Ended = wore off/was thrown off early. No
//! Failure case for these: there's no real "you resisted the stun/root/
//! fear" self-status line worth building yet (only an *avoided* attempt
//! exists for stun, which never lands at all -- see packs/eql.toml's
//! noise.stunned doc). Fear is deliberately rough: see
//! state.you_feared's own doc in the pack for why its ON list is a
//! curated, non-exhaustive set of real spell text rather than a clean
//! wiki category the way Stun/Root have.

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
    /// why: the spell that established the charm (your newest begun cast
    /// at confirm time) -- lets a wear-off line for some OTHER spell on
    /// the same name not read as a break; None when no recent cast
    pub spell: Option<String>,
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
    /// why: root/fear only -- the game drops these when their caster
    /// dies, but a death is AMBIGUOUS evidence: the name-keyed graph
    /// can't tell same-named instances apart, so the mob that just died
    /// may not be the one that cast it. Player's own spec: "maybe it
    /// needs a state of 'maybe?'". Resolved by the effect's own wear-off
    /// line, a fresh landing, or every enemy dying (nothing left that
    /// could be maintaining it).
    Uncertain,
}

impl MomentaryOutcome {
    fn label(self) -> &'static str {
        match self {
            MomentaryOutcome::Success => "success",
            MomentaryOutcome::Failure => "failure",
            MomentaryOutcome::Ended => "ended",
            MomentaryOutcome::Uncertain => "uncertain",
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
    /// why: "success" = landed/on, "ended" = wore off/off -- see this
    /// module's own doc for why CC never uses "failure"
    pub stun: Option<MomentaryDto>,
    pub root: Option<MomentaryDto>,
    pub fear: Option<MomentaryDto>,
    /// why: the game's generic "You lose control of yourself!" landing --
    /// shared by fear, charm-on-you, and captivate; the ender line
    /// reveals which. Its own square, since mapping it to Fear would be
    /// wrong ~35% of the time (measured: 127 landings in the reference
    /// log -- 82 ended "afraid", 18 "captivated", 14 "charmed").
    pub control: Option<ControlDto>,
}

/// why: MomentaryDto plus the probable enemy caster/spell -- mob casts
/// name their spells, so "Dragon Fear by A dracoliche" is knowable and
/// worth showing where a bare Ctrl square isn't
#[derive(Debug, Clone, Serialize)]
pub struct ControlDto {
    pub outcome: &'static str,
    pub since_ms: Millis,
    pub caster: Option<String>,
    pub spell: Option<String>,
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
        stun: ing.stun.map(|s| MomentaryDto {
            outcome: s.outcome.label(),
            since_ms: s.since_ms,
        }),
        root: ing.root.map(|s| MomentaryDto {
            outcome: s.outcome.label(),
            since_ms: s.since_ms,
        }),
        fear: ing.fear.map(|s| MomentaryDto {
            outcome: s.outcome.label(),
            since_ms: s.since_ms,
        }),
        control: ing.control.map(|s| ControlDto {
            outcome: s.outcome.label(),
            since_ms: s.since_ms,
            caster: ing.control_caster.clone(),
            spell: ing.control_spell.clone(),
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

    /// why: "<name> has been charmed." names no caster, so a charm is only
    /// YOURS once one of your own charm casts sits inside the retention
    /// window. These tests are about the charm's LIFECYCLE -- breaking,
    /// zoning, reaffirming -- so they need a charm that is actually yours;
    /// without the cast they were exercising the ownership hole instead.
    fn run_charmed(lines: &[&str]) -> Ingest {
        let mut all = vec!["[Tue Jul 28 15:00:58 2026] You begin casting Allure."];
        all.extend_from_slice(lines);
        run(&all)
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
        let ing = run_charmed(&["[Tue Jul 28 15:01:00 2026] an abhorrent has been charmed."]);
        let dto = status_effects(&ing);
        let c = dto.charm.expect("a charm should be tracked");
        assert_eq!(c.who, "an abhorrent");
        assert!(c.active);

        let ing = run_charmed(&[
            "[Tue Jul 28 15:01:00 2026] an abhorrent has been charmed.",
            "[Tue Jul 28 15:01:05 2026] Your Allure spell has worn off of an abhorrent.",
        ]);
        let dto = status_effects(&ing);
        assert!(!dto.charm.expect("still tracked, now inactive").active);
    }

    /// why: a charmed pet never follows across a zone line, and the
    /// break is often silent (no "worn off" line) -- zoning must clear
    /// it unconditionally, not wait for a confirmation that may never come
    #[test]
    fn zoning_breaks_an_active_charm_even_with_no_explicit_break_line() {
        let ing = run_charmed(&[
            "[Tue Jul 28 15:01:00 2026] an abhorrent has been charmed.",
            "[Tue Jul 28 15:01:05 2026] You have entered The Northern Desert of Ro.",
        ]);
        let dto = status_effects(&ing);
        assert!(
            !dto.charm.expect("still tracked, now inactive").active,
            "zoning must break an active charm even with no worn-off line"
        );
    }

    /// why: real bug shape this guards against -- state.charm_broken's
    /// pattern is generic ("Your <any spell> spell has worn off of
    /// <target>"), so an unrelated buff wearing off elsewhere must not
    /// false-clear a still-active charm on a different target
    #[test]
    fn an_unrelated_spell_wearing_off_a_different_target_does_not_clear_the_charm() {
        let ing = run_charmed(&[
            "[Tue Jul 28 15:01:00 2026] an abhorrent has been charmed.",
            "[Tue Jul 28 15:01:05 2026] Your Skin like Steel spell has worn off of You.",
        ]);
        let dto = status_effects(&ing);
        assert!(
            dto.charm.expect("still tracked").active,
            "a different target's buff wearing off must not clear this charm"
        );
    }

    /// why: a charmed pet can never legitimately land a hit on "You" --
    /// that alone proves the charm already broke silently (no worn-off
    /// line), whether it expired or a fresh mob reused the same name.
    #[test]
    fn a_charmed_name_attacking_you_breaks_the_charm_with_no_worn_off_line() {
        let ing = run_charmed(&[
            "[Tue Jul 28 15:01:00 2026] an abhorrent has been charmed.",
            "[Tue Jul 28 15:01:05 2026] an abhorrent hits You for 4 points of damage.",
        ]);
        let dto = status_effects(&ing);
        assert!(
            !dto.charm.expect("still tracked, now inactive").active,
            "the charmed name hitting You is proof the charm already broke"
        );
    }

    /// why: the flip side of the test above -- a charmed pet doing exactly
    /// what it's supposed to (hitting something that ISN'T you) must never
    /// be mistaken for evidence of a break
    #[test]
    fn a_charmed_pet_attacking_something_else_does_not_break_the_charm() {
        let ing = run_charmed(&[
            "[Tue Jul 28 15:01:00 2026] an abhorrent has been charmed.",
            "[Tue Jul 28 15:01:05 2026] an abhorrent hits a rat for 4 points of damage.",
        ]);
        let dto = status_effects(&ing);
        assert!(
            dto.charm.expect("still tracked").active,
            "a charmed pet fighting on your behalf must not clear its own charm"
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

    #[test]
    fn stun_lands_then_ends() {
        let ing = run(&["[Tue Jul 28 15:01:00 2026] You are stunned!"]);
        assert_eq!(
            status_effects(&ing).stun.expect("tracked").outcome,
            "success"
        );

        let ing = run(&[
            "[Tue Jul 28 15:01:00 2026] You are stunned!",
            "[Tue Jul 28 15:01:03 2026] You are no longer stunned.",
        ]);
        assert_eq!(status_effects(&ing).stun.expect("tracked").outcome, "ended");
    }

    /// why: real, confirmed second way stun ends -- resisting mid-effect,
    /// not just the natural "no longer stunned" expiry
    #[test]
    fn overcoming_a_stun_early_also_counts_as_ended() {
        let ing = run(&[
            "[Tue Jul 28 15:01:00 2026] You are stunned!",
            "[Tue Jul 28 15:01:01 2026] You overcome the stun!",
        ]);
        assert_eq!(status_effects(&ing).stun.expect("tracked").outcome, "ended");
    }

    #[test]
    fn root_lands_then_ends() {
        let ing = run(&["[Tue Jul 28 15:01:00 2026] You are ensnared."]);
        assert_eq!(
            status_effects(&ing).root.expect("tracked").outcome,
            "success"
        );

        let ing = run(&[
            "[Tue Jul 28 15:01:00 2026] You are ensnared.",
            "[Tue Jul 28 15:03:00 2026] You are no longer ensnared.",
        ]);
        assert_eq!(status_effects(&ing).root.expect("tracked").outcome, "ended");
    }

    /// why: the game drops root/fear when their caster dies, but a death
    /// is ambiguous (same-named mobs) -- player's spec: a possible-caster
    /// death goes to "maybe" (uncertain), never a confident clear, while
    /// other enemies still live
    #[test]
    fn a_possible_caster_death_marks_root_uncertain_not_ended() {
        let ing = run(&[
            "[Tue Jul 28 15:00:58 2026] a shimmering spirit begins casting a spell.",
            "[Tue Jul 28 15:01:00 2026] You are ensnared.",
            "[Tue Jul 28 15:01:05 2026] You hit a shimmering spirit for 50 points of damage.",
            "[Tue Jul 28 15:01:06 2026] a gust wisp hits YOU for 10 points of damage.",
            "[Tue Jul 28 15:01:10 2026] You have slain a shimmering spirit!",
        ]);
        assert_eq!(
            status_effects(&ing).root.expect("tracked").outcome,
            "uncertain",
            "caster-name death with another enemy alive is a maybe, not a clear"
        );
    }

    /// why: the other half of the spec -- "if a combat ends with all
    /// targets dead, it should clear all root/fear effects": nothing
    /// left alive that could be maintaining them
    #[test]
    fn all_enemies_dead_clears_root_and_fear_outright() {
        let ing = run(&[
            "[Tue Jul 28 15:01:00 2026] You are ensnared.",
            "[Tue Jul 28 15:01:01 2026] Your mind fills with fear.",
            "[Tue Jul 28 15:01:05 2026] You hit a shimmering spirit for 50 points of damage.",
            "[Tue Jul 28 15:01:10 2026] You have slain a shimmering spirit!",
        ]);
        let s = status_effects(&ing);
        assert_eq!(s.root.expect("tracked").outcome, "ended");
        assert_eq!(s.fear.expect("tracked").outcome, "ended");
    }

    /// why: stun is explicitly out of the death-clear ("not stun",
    /// player's own spec) -- it runs on its own short clock
    #[test]
    fn stun_is_untouched_by_enemy_deaths() {
        let ing = run(&[
            "[Tue Jul 28 15:01:00 2026] You are stunned!",
            "[Tue Jul 28 15:01:02 2026] You hit a shimmering spirit for 50 points of damage.",
            "[Tue Jul 28 15:01:03 2026] You have slain a shimmering spirit!",
        ]);
        assert_eq!(
            status_effects(&ing).stun.expect("tracked").outcome,
            "success"
        );
    }

    /// why: the generic lose-control landing lights its own square and
    /// each real ender resolves it -- charm-you/captivate via their own
    /// lines, fear via the shared "no longer afraid" (which ends both
    /// squares). All three measured real in the reference log
    #[test]
    fn lose_control_lands_then_resolves_by_each_real_ender() {
        let ing = run(&["[Tue Jul 28 15:01:00 2026] You lose control of yourself!"]);
        assert_eq!(
            status_effects(&ing).control.expect("tracked").outcome,
            "success"
        );

        for ender in [
            "[Tue Jul 28 15:01:20 2026] You are no longer charmed.",
            "[Tue Jul 28 15:01:20 2026] You are no longer captivated.",
            "[Tue Jul 28 15:01:20 2026] You are no longer afraid.",
            "[Tue Jul 28 15:01:20 2026] You have control of yourself again.",
        ] {
            let ing = run(&[
                "[Tue Jul 28 15:01:00 2026] You lose control of yourself!",
                ender,
            ]);
            assert_eq!(
                status_effects(&ing).control.expect("tracked").outcome,
                "ended",
                "ender: {ender}"
            );
        }
    }

    /// why: same caster-death ambiguity treatment root/fear get -- a
    /// possible-caster kill goes to maybe, all-enemies-dead clears
    #[test]
    fn lose_control_resolves_on_deaths_like_root_and_fear() {
        let ing = run(&[
            "[Tue Jul 28 15:00:58 2026] a shimmering spirit begins casting a spell.",
            "[Tue Jul 28 15:01:00 2026] You lose control of yourself!",
            "[Tue Jul 28 15:01:05 2026] You hit a shimmering spirit for 50 points of damage.",
            "[Tue Jul 28 15:01:06 2026] a gust wisp hits YOU for 10 points of damage.",
            "[Tue Jul 28 15:01:10 2026] You have slain a shimmering spirit!",
        ]);
        assert_eq!(
            status_effects(&ing).control.expect("tracked").outcome,
            "uncertain"
        );
    }

    /// why: player's spec -- "fear and charm depend on what the enemy is
    /// casting": the newest enemy cast whose spell classifies as the
    /// mechanic wins the attribution; a groupmate's interleaved heal
    /// (real shape in the log: Lifespike right before a landing) never does
    #[test]
    fn control_attributes_the_enemy_fear_cast_not_a_groupmate_heal() {
        let ing = run(&[
            "[Tue Jul 28 15:00:57 2026] A dracoliche begins casting Dragon Fear.",
            "[Tue Jul 28 15:00:58 2026] A dracoliche hits YOU for 100 points of damage.",
            "[Tue Jul 28 15:00:59 2026] Bravesirrobin begins casting Lifespike.",
            "[Tue Jul 28 15:01:00 2026] You lose control of yourself!",
        ]);
        let c = status_effects(&ing).control.expect("tracked");
        assert_eq!(c.outcome, "success");
        assert_eq!(c.spell.as_deref(), Some("Dragon Fear"));
        assert_eq!(c.caster.as_deref(), Some("A dracoliche"));
    }

    /// why: your own death ends every CC outright -- nothing survives it
    #[test]
    fn your_own_death_clears_all_cc() {
        let ing = run(&[
            "[Tue Jul 28 15:01:00 2026] You are ensnared.",
            "[Tue Jul 28 15:01:01 2026] You lose control of yourself!",
            "[Tue Jul 28 15:01:02 2026] You are stunned!",
            "[Tue Jul 28 15:01:10 2026] You have been slain by a sonic bat!",
        ]);
        let s = status_effects(&ing);
        assert_eq!(s.root.expect("tracked").outcome, "ended");
        assert_eq!(s.control.expect("tracked").outcome, "ended");
        assert_eq!(s.stun.expect("tracked").outcome, "ended");
    }

    /// why: an unrelated enemy dying while the KNOWN caster still lives
    /// must not shake the state -- the maybe is only for a death that
    /// could actually have been the caster's
    #[test]
    fn an_unrelated_death_with_a_known_living_caster_changes_nothing() {
        let ing = run(&[
            "[Tue Jul 28 15:00:58 2026] a shimmering spirit begins casting a spell.",
            "[Tue Jul 28 15:01:00 2026] You are ensnared.",
            "[Tue Jul 28 15:01:05 2026] a shimmering spirit hits YOU for 5 points of damage.",
            "[Tue Jul 28 15:01:06 2026] You hit a gust wisp for 50 points of damage.",
            "[Tue Jul 28 15:01:10 2026] You have slain a gust wisp!",
        ]);
        assert_eq!(
            status_effects(&ing).root.expect("tracked").outcome,
            "success",
            "the attributed caster is alive -- the root stands"
        );
    }

    /// why: real, curated set -- any of several different real fear
    /// spells' own landing text counts as ON, all sharing the one real
    /// wear-off line (see state.you_feared's own doc)
    #[test]
    fn fear_lands_from_any_curated_source_then_ends() {
        let ing = run(&["[Tue Jul 28 15:01:00 2026] You freeze in terror."]);
        assert_eq!(
            status_effects(&ing).fear.expect("tracked").outcome,
            "success"
        );

        let ing = run(&[
            "[Tue Jul 28 15:01:00 2026] Your mind fills with fear.",
            "[Tue Jul 28 15:01:30 2026] You are no longer afraid.",
        ]);
        assert_eq!(status_effects(&ing).fear.expect("tracked").outcome, "ended");
    }
}
