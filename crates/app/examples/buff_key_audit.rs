//! why: "verify all current spells (in current era) that they are key'd
//! correctly". For every in-era buff the tracker admits, this compares
//! the name key (rank_line) against the game's own grouping (conflict
//! components off spells_us.txt) and prints every disagreement: a name
//! line that holds spells which actually stack, a game line split across
//! names, and a kind row holding several stacking lines.
//! input: the install folder
//! run: cargo run -p eqlp-app --release --example buff_key_audit -- <install>

use eqlp_app::groupbuffs::{
    is_illusion, is_party_buff, is_recourse, is_self_buff, kind_of_with, rank_line, reachable,
    BuffKind,
};
use eqlp_app::spelldata::spells;
use eqlp_app::spelltimers::{entry_of, spell_file};
use std::collections::{BTreeMap, BTreeSet};

fn main() {
    let base = std::env::args()
        .nth(1)
        .expect("usage: buff_key_audit <install>");
    let file = spell_file(std::path::Path::new(&base));
    let ceiling = eqlp_app::gearplanner::era_ix(eqlp_app::gearplanner::CURRENT_ERA);
    let mut admitted: Vec<(&str, BuffKind, &str)> = Vec::new();
    for s in spells() {
        if is_recourse(s) || is_illusion(s) || !reachable(s, ceiling) {
            continue;
        }
        let side = if is_party_buff(s) {
            "party"
        } else if is_self_buff(s) {
            "self"
        } else {
            continue;
        };
        let Some(kind) = kind_of_with(s, Some(&file)) else {
            continue;
        };
        if entry_of(&file, &s.name).is_none() {
            continue;
        }
        admitted.push((&s.name, kind, side));
    }
    println!(
        "admitted in-era buffs with a file entry: {}",
        admitted.len()
    );

    // why: union-find over conflicts, per kind -- the game's own lines
    let mut n_name_split = 0;
    let mut n_game_split = 0;
    let mut n_row_multi = 0;
    let mut by_kind: BTreeMap<BuffKind, Vec<(&str, &str)>> = BTreeMap::new();
    for (name, kind, side) in &admitted {
        by_kind.entry(*kind).or_default().push((name, side));
    }
    for (kind, members) in &by_kind {
        let n = members.len();
        let mut parent: Vec<usize> = (0..n).collect();
        fn find(p: &mut [usize], i: usize) -> usize {
            if p[i] != i {
                let r = find(p, p[i]);
                p[i] = r;
            }
            p[i]
        }
        let entries: Vec<_> = members
            .iter()
            .map(|(nm, _)| entry_of(&file, nm).expect("admitted"))
            .collect();
        for i in 0..n {
            for j in (i + 1)..n {
                if entries[i].conflicts(&entries[j]) {
                    let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                    if a != b {
                        parent[a] = b;
                    }
                }
            }
        }
        let mut comps: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for i in 0..n {
            let r = find(&mut parent, i);
            comps.entry(r).or_default().push(i);
        }
        let mut names: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
        for (i, (nm, _)) in members.iter().enumerate() {
            names
                .entry(rank_line(nm))
                .or_default()
                .insert(find(&mut parent, i));
        }
        println!(
            "\n== {:<14} {} spells, {} game lines, {} name lines",
            kind.label(),
            n,
            comps.len(),
            names.len()
        );
        for (line, roots) in &names {
            if roots.len() > 1 {
                n_name_split += 1;
                let ms: Vec<String> = members
                    .iter()
                    .enumerate()
                    .filter(|(_, (nm, _))| rank_line(nm) == *line)
                    .map(|(i, (nm, _))| format!("{nm}[g{}]", find(&mut parent, i)))
                    .collect();
                println!(
                    "   NAME LINE SPANS {} GAME LINES: {line}: {}",
                    roots.len(),
                    ms.join(", ")
                );
            }
        }
        for idx in comps.values() {
            let lines: BTreeSet<String> = idx.iter().map(|&i| rank_line(members[i].0)).collect();
            if lines.len() > 1 {
                n_game_split += 1;
                let ms: Vec<String> = idx
                    .iter()
                    .map(|&i| format!("{}({})", members[i].0, members[i].1))
                    .collect();
                println!(
                    "   GAME LINE UNDER {} NAMES: {}",
                    lines.len(),
                    ms.join(", ")
                );
            }
        }
        let party_comps: BTreeSet<usize> = members
            .iter()
            .enumerate()
            .filter(|(_, (_, side))| *side == "party")
            .map(|(i, _)| find(&mut parent, i))
            .collect();
        if party_comps.len() > 1 {
            n_row_multi += 1;
            let mut per: Vec<String> = Vec::new();
            for r in &party_comps {
                let ms: Vec<&str> = members
                    .iter()
                    .enumerate()
                    .filter(|(i, (_, side))| *side == "party" && find(&mut parent, *i) == *r)
                    .map(|(_, (nm, _))| *nm)
                    .collect();
                per.push(format!("{{{}}}", ms.join(", ")));
            }
            println!(
                "   PARTY ROW HOLDS {} STACKING LINES: {}",
                party_comps.len(),
                per.join(" + ")
            );
        }
    }
    println!(
        "\nname lines spanning game lines: {n_name_split}   game lines split across names: {n_game_split}   party rows holding stacking lines: {n_row_multi}"
    );
}
