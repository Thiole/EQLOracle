//! why: derived fresh from Kind+State, not stored -- avoids a third
//! copy of truth; Unproven defaults Enemy, Charmed flips it either way

use crate::graph::Kind;
use crate::timeline::State;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Allegiance {
    Ally,
    Enemy,
}

impl Allegiance {
    /// why: pure and cheap -- called per-query, never cached
    pub fn of(kind: Kind, state: State) -> Allegiance {
        let base = match kind {
            Kind::Player | Kind::Pet => Allegiance::Ally,
            Kind::Unproven => Allegiance::Enemy,
        };
        if state == State::Charmed {
            base.flipped()
        } else {
            base
        }
    }

    pub fn flipped(self) -> Allegiance {
        match self {
            Allegiance::Ally => Allegiance::Enemy,
            Allegiance::Enemy => Allegiance::Ally,
        }
    }

    pub fn is_enemy(self) -> bool {
        self == Allegiance::Enemy
    }

    pub fn name(self) -> &'static str {
        match self {
            Allegiance::Ally => "ally",
            Allegiance::Enemy => "enemy",
        }
    }
}
