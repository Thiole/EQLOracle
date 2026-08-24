//! why: seeded generators for property tests, seed from EQLP_SEED

use std::sync::atomic::{AtomicU64, Ordering};

/// why: deterministic, replayable from the printed seed
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(6364136223846793005).wrapping_add(1))
    }

    /// why: env seed, else a fixed fallback for bare `cargo test`
    pub fn from_env() -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let base = std::env::var("EQLP_SEED")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0xC0FFEE);
        // why: distinct stream per instance, parallel tests don't collide
        Rng::new(base ^ N.fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed))
    }

    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let x = self.0;
        (x >> 18) ^ x
    }

    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next_u64() % n as u64) as usize
        }
    }

    pub fn range(&mut self, lo: u64, hi: u64) -> u64 {
        if hi <= lo {
            lo
        } else {
            lo + self.next_u64() % (hi - lo)
        }
    }

    pub fn bool(&mut self, pct: u64) -> bool {
        self.next_u64() % 100 < pct
    }

    pub fn pick<'a, T>(&mut self, v: &'a [T]) -> &'a T {
        &v[self.below(v.len())]
    }

    pub fn bytes(&mut self, len: usize) -> Vec<u8> {
        (0..len).map(|_| (self.next_u64() & 0xff) as u8).collect()
    }
}

pub const ACTORS: &[&str] = &[
    "You",
    "Kaeus",
    "Bravesirrobin",
    "Dippinsauce",
    "Sidhe",
    "Balanque",
    "a decaying skeleton",
    "an abhorrent",
    "Footman of V`Zher",
    "the Ghoul Lord",
];
pub const VERBS: &[&str] = &[
    "slashes",
    "bashes",
    "kicks",
    "hits",
    "crushes",
    "pierces",
    "backstabs",
    "claws",
];
pub const FLAGS: &[&str] = &[
    "",
    " (Critical)",
    " (Riposte)",
    " (Rampage)",
    " (Critical Double Bow Shot)",
];
pub const SPELLS: &[&str] = &[
    "Ice Comet",
    "Garrison's Mighty Mana Shock",
    "Lifetap",
    "Minor Healing",
    "Blessing of the Squire",
    "Elemental Maelstrom",
];

/// why: pairs a log line with the facts it encodes, for assertions
pub struct GenLine {
    pub line: String,
    pub ts_secs: i64,
    pub actor: String,
    pub target: String,
    pub amount: u64,
    pub kind: &'static str,
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

pub fn stamp(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    // civil-from-days
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "[{} {} {:02} {:02}:{:02}:{:02} {}]",
        DAYS[(days.rem_euclid(7)) as usize],
        MONTHS[(m - 1) as usize],
        d,
        tod / 3600,
        (tod % 3600) / 60,
        tod % 60,
        y
    )
}

/// why: random damage line, mixes melee/spell/DoT/damage-shield forms
pub fn damage_line(rng: &mut Rng, ts: i64) -> GenLine {
    let actor = rng.pick(ACTORS).to_string();
    let target = rng.pick(ACTORS).to_string();
    let amount = rng.range(1, 3000);
    let pts = if amount == 1 { "point" } else { "points" };
    let (body, kind) = match rng.below(4) {
        0 => (
            format!(
                "{} {} {} for {} {} of damage.{}",
                actor,
                rng.pick(VERBS),
                target,
                amount,
                pts,
                rng.pick(FLAGS)
            ),
            "melee",
        ),
        1 => (
            format!(
                "{} hit {} for {} points of {} damage by {}.",
                actor,
                target,
                amount,
                rng.pick(&["magic", "fire", "cold", "poison"]),
                rng.pick(SPELLS)
            ),
            "spell",
        ),
        2 => (
            format!(
                "{} has taken {} damage from {} by {}.",
                target,
                amount,
                rng.pick(SPELLS),
                actor
            ),
            "dot",
        ),
        _ => (
            format!(
                "{} is burned by {}'s flames for {} points of non-melee damage.",
                target, actor, amount
            ),
            "ds",
        ),
    };
    GenLine {
        line: format!("{} {}", stamp(ts), body),
        ts_secs: ts,
        actor,
        target,
        amount,
        kind,
    }
}

/// why: synthetic session, random encounters + participants + noise lines
pub fn session(rng: &mut Rng, encounters: usize) -> (String, Vec<GenLine>) {
    let mut out = String::new();
    let mut facts = Vec::new();
    let mut t = 1_754_514_873i64;
    for _ in 0..encounters {
        let target = rng.pick(ACTORS).to_string();
        for _ in 0..rng.range(2, 25) {
            t += rng.range(0, 3) as i64;
            let mut g = damage_line(rng, t);
            g.target = target.clone();
            let body = g
                .line
                .split_once("] ")
                .map(|(_, rest)| rest)
                .unwrap_or("")
                .to_string();
            let _ = body;
            out.push_str(&g.line);
            out.push_str("\r\n");
            facts.push(g);
            if rng.bool(10) {
                out.push_str(&format!("{} You feel a sense of loss.\r\n", stamp(t)));
            }
        }
        t += rng.range(1, 6) as i64;
        out.push_str(&format!(
            "{} {} has been slain by Kaeus!\r\n",
            stamp(t),
            target
        ));
        t += rng.range(20, 300) as i64;
    }
    (out, facts)
}
