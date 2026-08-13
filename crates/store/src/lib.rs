//! In-memory columnar event store. One source of truth; aggregates are queries.
//!
//! Design notes: `docs/design/store.md`

pub mod ability;
pub mod query;
pub mod store;

pub use ability::{tag, Abilities, Ability, AbilityId, Interner, Sym, Tags};
pub use query::{by_ability, by_actor, dps_window, roll_up_by_tag, total, AbilityRow, Filter};
pub use store::{flag, Encounter, EncounterId, EventKind, Flags, Store, NO_ENCOUNTER};
