//! why: verify whether "damaged the same real encounter as You" actually
//! correlates with real group membership (not just "same target name" --
//! a public zone can have several live mobs sharing one name). One pass
//! over the store grouped by real EncounterId, not name matching. v2:
//! also classifies each co-damager's Allegiance at time-of-hit (same
//! Kind+State logic `Ingest::is_ally` uses) -- v1's raw actor list was
//! dominated by mob/charmed-pet noise (see out.tsv analysis), this
//! isolates the confirmed-ally signal from it.
//! input: path to a real combat log
//! output: one line per (encounter start_ms, other real damage-dealer, ally|enemy)
//! run: cargo run -p eqlp-app --release --example group_evidence_check -- <log> > out.tsv
use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;
use eqlp_session::{Allegiance, State};
use eqlp_store::EventKind;
use std::collections::{HashMap, HashSet};
use std::io::Write;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: group_evidence_check <log>");
    let engine = build_engine().expect("pack builds");
    let bytes = std::fs::read(&path).expect("read log");
    let lines: Vec<&[u8]> = framed_lines(&bytes);
    let mut ing = Ingest::default();
    backfill_lines(
        &mut ing,
        &engine,
        &lines,
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4),
    );

    // why: interned Sym, not a cloned String per row -- keeps this
    // O(store length) pass cheap on a multi-million-row store. Target
    // must be the encounter's own anchor specifically -- a plain
    // `enc == this encounter` match also picks up the anchor's own
    // outgoing hits on party members (incoming damage rows share the
    // same enc id), which would count the enemy itself as a "dealer".
    // why: one global pass, not one scan per encounter (quadratic-shaped
    // -- this app's own history has a real regression from exactly that
    // mistake, see monsters.rs's doc). anchor_of first, single lookup per row.
    let anchor_of: HashMap<u32, eqlp_store::Sym> = ing
        .store
        .encounters
        .iter()
        .map(|e| (e.id.0, e.target))
        .collect();
    // why: (ts, actor sym) not just actor sym -- need time-of-hit for the
    // allegiance-at-that-moment check below (charm state changes within a fight)
    let mut dealers: HashMap<u32, HashSet<(Millis, u32)>> = HashMap::new();
    for i in 0..ing.store.len() {
        if ing.store.kind[i] != EventKind::Damage {
            continue;
        }
        let enc = ing.store.enc[i];
        let Some(&anchor) = anchor_of.get(&enc) else {
            continue;
        };
        if ing.store.target[i] != anchor {
            continue;
        }
        dealers
            .entry(enc)
            .or_default()
            .insert((ing.store.ts[i], ing.store.actor[i].0));
    }

    // why: same Kind+State logic Ingest::is_ally uses, replicated here
    // since it's private to eqlp-app -- see that fn's own doc for why
    // "ally as of ts, not forever" matters (a charmed player/pet reads enemy).
    let classify_at = |ing: &Ingest, sym: u32, ts: Millis| -> (eqlp_session::Kind, Allegiance) {
        let name = ing.store.name(eqlp_store::Sym(sym));
        let kind = if name.eq_ignore_ascii_case("you") {
            eqlp_session::Kind::Player
        } else {
            ing.encounters.entities.kind(name)
        };
        let canonical = ing.encounters.entities.display_name(name);
        let state = ing
            .store
            .names
            .get(canonical)
            .and_then(|s| ing.timeline.state_at(s.0, ts))
            .map(|(s, _)| s)
            .unwrap_or(State::Engaged);
        (kind, Allegiance::of(kind, state))
    };

    let you = ing.store.names.get("You").map(|s| s.0);
    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::new(stdout.lock());
    for e in &ing.store.encounters {
        let Some(hits) = dealers.get(&e.id.0) else {
            continue;
        };
        let Some(you_sym) = you else { continue };
        if !hits.iter().any(|&(_, s)| s == you_sym) {
            continue;
        }
        for &(ts, s) in hits {
            if s == you_sym {
                continue;
            }
            let name = ing.store.name(eqlp_store::Sym(s));
            let (kind, allegiance) = classify_at(&ing, s, ts);
            let kind_str = match kind {
                eqlp_session::Kind::Player => "player",
                eqlp_session::Kind::Pet => "pet",
                eqlp_session::Kind::Unproven => "unproven",
            };
            writeln!(
                out,
                "{}\t{}\t{}\t{}",
                e.start_ms,
                name,
                if allegiance.is_enemy() {
                    "enemy"
                } else {
                    "ally"
                },
                kind_str,
            )
            .ok();
        }
    }
}

type Millis = i64;
