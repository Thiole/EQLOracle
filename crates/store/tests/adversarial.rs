//! why: adversarial coverage for the store's eviction seam -- unwired
//! at runtime today, but public API whose id-stability contract must
//! hold the day it's wired. Attacks: dangling EncounterId resolution,
//! range shifting, and post-eviction appends.

use eqlp_store::{tag, EventKind, Store};

#[test]
fn eviction_leaves_dangling_ids_unresolvable_and_survivors_intact() {
    let mut s = Store::default();
    let a = s.sym("You");
    let m1 = s.sym("mob one");
    let m2 = s.sym("mob two");
    let ab = s.ability_id("Burst", tag::SPELL);

    let i0 = s.push(1_000, EventKind::Damage, a, m1, ab, 10, 0, 0, 0);
    let e1 = s.open_encounter(m1, 1_000, i0, None);
    s.close_encounter(e1, 2_000, true, false);

    let i1 = s.push(3_000, EventKind::Damage, a, m2, ab, 20, 0, 1, 0);
    let e2 = s.open_encounter(m2, 3_000, i1, None);
    s.extend_encounter(e2, i1);

    s.evict_before_encounter(1);

    // why: the evicted id must resolve to None, never to the survivor
    assert!(s.encounter(e1).is_none(), "evicted id must not resolve");
    let surv = s.encounter(e2).expect("survivor resolves");
    // why: ranges shifted with the drained rows -- the survivor's range
    // must still point at ITS row, not the evicted one's slot
    assert_eq!(s.amount[surv.range().start], 20);
    assert_eq!(s.len(), 1);

    // why: appends after eviction keep working and keep ids monotonic
    let i2 = s.push(4_000, EventKind::Damage, a, m2, ab, 30, 0, 1, 0);
    s.extend_encounter(e2, i2);
    let surv = s.encounter(e2).expect("still resolves");
    assert_eq!(surv.range().len(), 2);
    let e3 = s.open_encounter(m1, 5_000, i2, None);
    assert!(e3.0 > e2.0, "ids never renumber, even across eviction");
}
