//! Ally vs. enemy, derived rather than stored.
//!
//! The log gives no direct enemy/ally flag -- `graph::Kind` only carries
//! identity certainty (`Player`/`Pet`/`Unproven`), and `timeline::State`
//! only carries combat status (engaged/mezzed/charmed/dead/lost). Neither
//! alone answers "whose side is this on right now", and storing the answer
//! separately would be a third copy of information the other two already
//! hold -- exactly the "two places of truth" `Tracker` is flagged for
//! elsewhere. So `Allegiance` isn't stored anywhere; it's computed fresh
//! from whatever `Kind` and `State` already say, the same way current combat
//! state is a query over `Timeline`, not a field.
//!
//! `Unproven` defaults to `Enemy` -- consistent with `Kind`'s own doc
//! comment ("most are NPCs"), and with the same acknowledged risk: a
//! teammate who hasn't spoken on a player-only channel yet reads as an
//! enemy until they do.
//!
//! Charm is the one thing that flips it, and it's genuinely temporary --
//! `State::Charmed` reverts to `Engaged` the moment `state.charm_broken`
//! (or its self-only equivalent) lands, and `Allegiance::of` re-derives from
//! whatever `Timeline` says at query time, so there's nothing to un-flip by
//! hand. One rule handles it in both directions: a charmed `Unproven` mob
//! (the common case -- an enchanter's charm pet) becomes a temporary `Ally`,
//! and a charmed `Player`/`Pet` (rarer, but the log doesn't distinguish who
//! cast the charm -- see `state.charmed`'s note on unattributed casters)
//! becomes a temporary `Enemy`.

use crate::graph::Kind;
use crate::timeline::State;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Allegiance {
    Ally,
    Enemy,
}

impl Allegiance {
    /// Derive allegiance from identity and current combat state. Pure and
    /// cheap -- safe to call per-entity, per-query, rather than cached.
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
