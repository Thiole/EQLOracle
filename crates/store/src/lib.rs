//! In-memory columnar event store. One source of truth; aggregates are queries.
//!
//! Design notes: `docs/design/store.md`

pub mod ability;
pub mod query;
pub mod score;
pub mod store;

pub use ability::{tag, Abilities, Ability, AbilityId, Interner, Sym, Tags};
pub use query::{
    by_ability, by_actor, by_target_and_ability, dps_window, roll_up_by_tag, total, AbilityRow,
    Filter,
};
pub use score::{score_parse, AbilityScore, GearModifiers, ParseScore};
pub use store::{flag, Encounter, EncounterId, EventKind, Flags, Store, NO_ENCOUNTER};
