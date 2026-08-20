//! Tests for `allegiance`. Kept out of the production module by convention:
//! src/ contains shipping code only.

use eqlp_session::allegiance::Allegiance;
use eqlp_session::graph::Kind;
use eqlp_session::timeline::State;

#[test]
fn player_and_pet_default_ally() {
    assert_eq!(
        Allegiance::of(Kind::Player, State::Engaged),
        Allegiance::Ally
    );
    assert_eq!(Allegiance::of(Kind::Pet, State::Engaged), Allegiance::Ally);
}

#[test]
fn unproven_defaults_enemy() {
    assert_eq!(
        Allegiance::of(Kind::Unproven, State::Engaged),
        Allegiance::Enemy
    );
}

#[test]
fn mezzed_does_not_flip_allegiance() {
    // Incapacitated, not turned -- a mezzed mob is still hostile.
    assert_eq!(
        Allegiance::of(Kind::Unproven, State::Mezzed),
        Allegiance::Enemy
    );
}

#[test]
fn charm_flips_an_unproven_mob_to_ally() {
    assert_eq!(
        Allegiance::of(Kind::Unproven, State::Charmed),
        Allegiance::Ally
    );
}

#[test]
fn charm_flips_a_player_or_pet_to_enemy() {
    assert_eq!(
        Allegiance::of(Kind::Player, State::Charmed),
        Allegiance::Enemy
    );
    assert_eq!(Allegiance::of(Kind::Pet, State::Charmed), Allegiance::Enemy);
}

#[test]
fn dead_and_lost_keep_their_base_allegiance() {
    assert_eq!(
        Allegiance::of(Kind::Unproven, State::Dead),
        Allegiance::Enemy
    );
    assert_eq!(Allegiance::of(Kind::Player, State::Lost), Allegiance::Ally);
}
