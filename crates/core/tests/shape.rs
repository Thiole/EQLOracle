//! Tests for `shape`. Kept out of the production module by
//! convention: src/ contains shipping code only.

use eqlp_core::shape::{shape, ShapeMode};

fn s(x: &[u8], m: ShapeMode) -> String {
    String::from_utf8(shape(x, m)).unwrap()
}

#[test]
fn digits_collapse() {
    assert_eq!(
        s(b"You hit a rat for 12 points of damage.", ShapeMode::Digits),
        "You hit a rat for # points of damage."
    );
}

#[test]
fn whitespace_is_normalised() {
    assert_eq!(s(b"  a   b \t c  ", ShapeMode::Digits), "a b c");
}

#[test]
fn multiword_names_collapse_to_one_placeholder() {
    let m = ShapeMode::Aggressive;
    assert_eq!(s(b"You begin casting Lifetap.", m), "You begin casting @.");
    assert_eq!(s(b"You begin casting Minor Healing.", m), "You begin casting @.");
    assert_eq!(
        s(b"You begin casting Garrison's Mighty Mana Shock VI.", m),
        "You begin casting @."
    );
}

#[test]
fn connectives_inside_names_are_bridged() {
    let m = ShapeMode::Aggressive;
    // Real lines that previously split into three separate shapes.
    assert_eq!(
        s(b"Bravesirrobin slashes Footman of V`Zher for 36 points of damage.", m),
        "@ slashes @ for # points of damage."
    );
    assert_eq!(
        s(b"Kaeus slashes Splitpaw Sentry for 13 points of damage.", m),
        "@ slashes @ for # points of damage."
    );
    assert_eq!(
        s(b"Rammu healed himself for 2 hit points by Blessing of the Squire.", m),
        "@ healed himself for # hit points by @."
    );
}

#[test]
fn pronouns_are_not_names() {
    let m = ShapeMode::Aggressive;
    assert_eq!(s(b"Your Flowing Black Robe flickers.", m), "Your @ flickers.");
    assert_ne!(
        s(b"Your robe glows.", m),
        s(b"Kaeus robe glows.", m),
        "self and other must never merge"
    );
}

#[test]
fn leading_punctuation_ends_a_run() {
    assert_eq!(
        s(b"Your Flowing Black Robe (Exaltation) flickers with a pale light.", ShapeMode::Aggressive),
        "Your @ (@) flickers with a pale light."
    );
    assert_eq!(
        s(b"Vhenwheel healed himself for 0 (4) hit points by Lifetap.", ShapeMode::Aggressive),
        "@ healed himself for # (#) hit points by @."
    );
}

#[test]
fn punctuation_is_a_real_boundary() {
    assert_eq!(
        s(b"Kabann slashes Innoruuk, the Prince of Hate for 43 points.", ShapeMode::Aggressive),
        "@ slashes @, the @ for # points."
    );
}

#[test]
fn default_mode_keeps_the_actor_distinct() {
    let m = ShapeMode::DigitsAndNames;
    assert_ne!(s(b"You hit a rat for 3 points.", m), s(b"Braxus hit a rat for 3 points.", m));
}

#[test]
fn never_panics_on_bytes() {
    for len in 0..256 {
        let junk: Vec<u8> = (0..len).map(|i| (i as u8).wrapping_mul(37)).collect();
        for m in [ShapeMode::Digits, ShapeMode::DigitsAndNames, ShapeMode::Aggressive] {
            let _ = shape(&junk, m);
        }
    }
}
