//! why: how the Group Buff Tracker orders its rows for a class combo
//! input: <class> <class> <class>
fn main() {
    let mine: Vec<String> = std::env::args().skip(1).collect();
    let mut rows: Vec<(u32, &str)> = [
        eqlp_app::groupbuffs::BuffKind::ManaRegen,
        eqlp_app::groupbuffs::BuffKind::Haste,
        eqlp_app::groupbuffs::BuffKind::Hp,
        eqlp_app::groupbuffs::BuffKind::Ac,
        eqlp_app::groupbuffs::BuffKind::HpRegen,
        eqlp_app::groupbuffs::BuffKind::Resist,
        eqlp_app::groupbuffs::BuffKind::Movement,
        eqlp_app::groupbuffs::BuffKind::Attack,
        eqlp_app::groupbuffs::BuffKind::Strength,
        eqlp_app::groupbuffs::BuffKind::Dexterity,
        eqlp_app::groupbuffs::BuffKind::Stamina,
        eqlp_app::groupbuffs::BuffKind::Agility,
        eqlp_app::groupbuffs::BuffKind::DamageShield,
    ]
    .into_iter()
    .map(|k| (eqlp_app::groupbuffs::relevance(k, &mine), k.label()))
    .filter(|(r, _)| *r > 0)
    .collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.0));
    println!("{mine:?}");
    for (r, l) in rows {
        println!("  {r:>4}  {l}");
    }
}
