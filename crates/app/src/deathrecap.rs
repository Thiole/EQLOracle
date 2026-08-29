//! why: "why did I just die" -- the trailing window of incoming damage,
//! avoided swings, and heals received before a player death, grouped by
//! source+ability. Every input is already columnar in the store; this is
//! a read-only query, no new ingest state.
//!
//! Deaths come from the Timeline ("You" -> State::Dead, Observed --
//! death.you_died's own line), NOT store rows: EventKind::Death rows are
//! never pushed (checked -- record_death only touches the graph and the
//! timeline). Window is 30s by the player's own spec, not the 15s first
//! proposed.

use crate::ingest::Ingest;
use eqlp_session::{Cause, State};
use eqlp_source::Millis;
use eqlp_store::{flag, EventKind};
use serde::Serialize;
use std::collections::HashMap;

pub const RECAP_WINDOW_MS: Millis = 30_000;

/// why: one source+ability line of the recap table, worst-first sortable
#[derive(Debug, Clone, Serialize)]
pub struct RecapRowDto {
    pub source: String,
    pub ability: String,
    pub total: u64,
    pub hits: u64,
    pub max_hit: u64,
    /// why: fully-avoided swings from this same source+ability -- shown
    /// as context ("you dodged 3 of these"), not damage
    pub avoided: u64,
}

/// why: the single hit that landed last before the death line -- the
/// killing blow as far as the log can tell (same second at worst)
#[derive(Debug, Clone, Serialize)]
pub struct KillingBlowDto {
    pub source: String,
    pub ability: String,
    pub amount: u64,
    pub ts_ms: Millis,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeathRecapDto {
    pub death_ts_ms: Millis,
    pub window_ms: Millis,
    pub killing_blow: Option<KillingBlowDto>,
    /// why: incoming damage rows, biggest total first
    pub incoming: Vec<RecapRowDto>,
    /// why: heals received in the same window, biggest first -- "was
    /// anyone even healing me" is half the question
    pub heals: Vec<RecapRowDto>,
    pub total_incoming: u64,
    pub total_healed: u64,
}

/// why: every observed player death this session, oldest first -- the
/// picker list; recap_at answers for any one of them
pub fn death_timestamps(ing: &Ingest) -> Vec<Millis> {
    let Some(you) = ing.store.names.get("You") else {
        return Vec::new();
    };
    ing.timeline
        .transitions_of(you.0)
        .iter()
        .filter(|t| t.state == State::Dead && t.cause == Cause::Observed)
        .map(|t| t.ts)
        .collect()
}

/// why: recap for the death at exactly `death_ts` (a value from
/// death_timestamps), or the most recent death when None. None result =
/// no player death observed this session at all.
pub fn recap(ing: &Ingest, death_ts: Option<Millis>) -> Option<DeathRecapDto> {
    let deaths = death_timestamps(ing);
    let death_ts_ms = match death_ts {
        Some(t) => *deaths.iter().find(|&&d| d == t)?,
        None => *deaths.last()?,
    };
    let you = ing.store.names.get("You")?;
    let from = death_ts_ms - RECAP_WINDOW_MS;

    // why: (source, ability) -> damage row; avoided merged onto the same
    // key so "hit you for 800 twice, you dodged a third" reads as one line
    let mut dmg: HashMap<(String, String), RecapRowDto> = HashMap::new();
    let mut heals: HashMap<(String, String), RecapRowDto> = HashMap::new();
    let mut killing_blow: Option<KillingBlowDto> = None;
    let mut total_incoming = 0u64;
    let mut total_healed = 0u64;

    let a = ing.store.ts.partition_point(|&t| t < from);
    let b = ing.store.ts.partition_point(|&t| t <= death_ts_ms);
    for i in a..b {
        if ing.store.target[i] != you {
            continue;
        }
        let source = ing.store.name(ing.store.actor[i]).to_string();
        let ability = ing.store.ability_name(ing.store.ability[i]).to_string();
        match ing.store.kind[i] {
            EventKind::Damage => {
                let amount = ing.store.amount[i];
                total_incoming += amount;
                let e = dmg
                    .entry((source.clone(), ability.clone()))
                    .or_insert_with(|| RecapRowDto {
                        source: source.clone(),
                        ability: ability.clone(),
                        total: 0,
                        hits: 0,
                        max_hit: 0,
                        avoided: 0,
                    });
                e.total += amount;
                e.hits += 1;
                e.max_hit = e.max_hit.max(amount);
                // why: last damage row at/before the death line wins --
                // rows are time-ordered, so plain overwrite is that
                killing_blow = Some(KillingBlowDto {
                    source,
                    ability,
                    amount,
                    ts_ms: ing.store.ts[i],
                });
            }
            EventKind::Miss if ing.store.flags[i] & flag::MITIGATED != 0 => {
                let e = dmg
                    .entry((source.clone(), ability.clone()))
                    .or_insert_with(|| RecapRowDto {
                        source,
                        ability,
                        total: 0,
                        hits: 0,
                        max_hit: 0,
                        avoided: 0,
                    });
                e.avoided += 1;
            }
            EventKind::Heal => {
                let amount = ing.store.amount[i];
                total_healed += amount;
                let e = heals
                    .entry((source.clone(), ability.clone()))
                    .or_insert_with(|| RecapRowDto {
                        source,
                        ability,
                        total: 0,
                        hits: 0,
                        max_hit: 0,
                        avoided: 0,
                    });
                e.total += amount;
                e.hits += 1;
                e.max_hit = e.max_hit.max(amount);
            }
            _ => {}
        }
    }

    let mut incoming: Vec<RecapRowDto> = dmg.into_values().collect();
    incoming.sort_by_key(|r| std::cmp::Reverse(r.total));
    let mut heals: Vec<RecapRowDto> = heals.into_values().collect();
    heals.sort_by_key(|r| std::cmp::Reverse(r.total));

    Some(DeathRecapDto {
        death_ts_ms,
        window_ms: RECAP_WINDOW_MS,
        killing_blow,
        incoming,
        heals,
        total_incoming,
        total_healed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::{backfill_lines, framed_lines};
    use crate::parser::build_engine;

    fn run(log: &str) -> Ingest {
        let engine = build_engine().expect("pack builds");
        let bytes = log.as_bytes();
        let lines = framed_lines(bytes);
        let mut ing = Ingest::default();
        backfill_lines(&mut ing, &engine, &lines, 1);
        ing
    }

    #[test]
    fn no_death_yet_reports_none() {
        let ing =
            run("[Tue Jul 28 15:01:00 2026] Guard Fintran hits YOU for 10 points of damage.\n");
        assert!(recap(&ing, None).is_none());
        assert!(death_timestamps(&ing).is_empty());
    }

    #[test]
    fn a_death_recaps_the_incoming_damage_and_names_the_killing_blow() {
        let ing = run(concat!(
            "[Tue Jul 28 15:01:00 2026] Guard Fintran hits YOU for 10 points of damage.\n",
            "[Tue Jul 28 15:01:05 2026] Guard Fintran hits YOU for 25 points of damage.\n",
            "[Tue Jul 28 15:01:06 2026] You have been slain by Guard Fintran!\n",
        ));
        let r = recap(&ing, None).expect("one death observed");
        assert_eq!(r.window_ms, RECAP_WINDOW_MS);
        assert_eq!(r.total_incoming, 35);
        assert_eq!(r.incoming.len(), 1);
        assert_eq!(r.incoming[0].hits, 2);
        assert_eq!(r.incoming[0].max_hit, 25);
        let kb = r.killing_blow.expect("last hit is the blow");
        assert_eq!(kb.source, "Guard Fintran");
        assert_eq!(kb.amount, 25);
    }

    #[test]
    fn damage_older_than_the_window_is_excluded() {
        let ing = run(concat!(
            "[Tue Jul 28 15:00:00 2026] Guard Fintran hits YOU for 500 points of damage.\n",
            "[Tue Jul 28 15:01:05 2026] Guard Fintran hits YOU for 25 points of damage.\n",
            "[Tue Jul 28 15:01:06 2026] You have been slain by Guard Fintran!\n",
        ));
        // why: 15:00:00 is 66s before the death -- outside the 30s window
        let r = recap(&ing, None).expect("death observed");
        assert_eq!(r.total_incoming, 25, "the 500 landed over 30s earlier");
    }

    #[test]
    fn heals_received_in_the_window_are_reported_separately() {
        let ing = run(concat!(
            "[Tue Jul 28 15:01:00 2026] Guard Fintran hits YOU for 25 points of damage.\n",
            "[Tue Jul 28 15:01:02 2026] Dippinsauce healed you for 40 hit points by Minor Healing.\n",
            "[Tue Jul 28 15:01:06 2026] You have been slain by Guard Fintran!\n",
        ));
        let r = recap(&ing, None).expect("death observed");
        assert_eq!(r.total_healed, 40);
        assert_eq!(r.heals.len(), 1);
        assert_eq!(r.heals[0].source, "Dippinsauce");
    }

    #[test]
    fn a_specific_earlier_death_can_be_recapped_by_timestamp() {
        let ing = run(concat!(
            "[Tue Jul 28 15:01:00 2026] Guard Fintran hits YOU for 10 points of damage.\n",
            "[Tue Jul 28 15:01:06 2026] You have been slain by Guard Fintran!\n",
            "[Tue Jul 28 16:00:00 2026] an evil eye hits YOU for 99 points of damage.\n",
            "[Tue Jul 28 16:00:05 2026] You have been slain by an evil eye!\n",
        ));
        let deaths = death_timestamps(&ing);
        assert_eq!(deaths.len(), 2);
        let first = recap(&ing, Some(deaths[0])).expect("first death");
        assert_eq!(first.total_incoming, 10);
        let latest = recap(&ing, None).expect("latest death");
        assert_eq!(latest.total_incoming, 99);
    }
}
