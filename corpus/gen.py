#!/usr/bin/env python3
"""Deterministic synthetic log generator.

Stands in for a real log until we have one. Deliberately mixes in ~25% of lines
the pack does not cover, because a corpus that is 100% matchable makes both the
benchmark and the coverage number lie.

    python3 corpus/gen.py 200000 > corpus/synthetic.log
"""
import random
import sys
import time

N = int(sys.argv[1]) if len(sys.argv) > 1 else 50_000
rng = random.Random(20260809)

PLAYERS = ["Kenkyo", "Braxus", "Nimly", "Torvald", "Sylwen", "Grimfang", "Ashka", "Doryn"]
MOBS = ["a decaying skeleton", "a giant rat", "an orc pawn", "a young kodiak",
        "a gnoll scout", "the Ghoul Lord", "a lesser mummy", "a kobold shaman"]
VERBS = ["slash", "crush", "pierce", "bash", "kick", "hit", "bite", "claw"]
ZONES = ["Greater Faydark", "Befallen", "Lower Guk", "Blackburrow", "Oasis of Marr"]
ITEMS = ["Rusty Short Sword", "Bone Chips", "Fine Steel Long Sword", "Words of Odus"]
SPELLS = ["Complete Heal", "Lightning Bolt", "Burst of Flame", "Superior Healing"]
CHATS = ["inc east", "train to zone", "med break", "need a port", "camp check?"]

# Lines the pack does not cover — the honest 25%.
NOISE = [
    "You feel a sense of loss.",
    "Your target is too far away, get closer!",
    "LOADING, PLEASE WAIT...",
    "You can't reach that.",
    "There is no one else nearby!",
    "Welcome to EverQuest!",
    "You have entered an area where levitation effects do not function.",
    "Your spell fizzles!",
    "Your faction standing with Freeport Militia has gotten worse.",
]

MONTHS = ["Jan","Feb","Mar","Apr","May","Jun","Jul","Aug","Sep","Oct","Nov","Dec"]
DAYS = ["Mon","Tue","Wed","Thu","Fri","Sat","Sun"]


def stamp(t):
    lt = time.gmtime(t)
    return "[%s %s %02d %02d:%02d:%02d %d]" % (
        DAYS[lt.tm_wday], MONTHS[lt.tm_mon - 1], lt.tm_mday,
        lt.tm_hour, lt.tm_min, lt.tm_sec, lt.tm_year)


def body():
    r = rng.random()
    p, m, v = rng.choice(PLAYERS), rng.choice(MOBS), rng.choice(VERBS)
    if r < 0.42:
        src, dst = (("You", m) if rng.random() < 0.5 else (m, "YOU"))
        vb = v if src == "You" else v + "s"
        return "%s %s %s for %s points of damage." % (src, vb, dst, rng.randint(1, 4000))
    if r < 0.52:
        return "%s was hit by non-melee for %d points of damage." % (m, rng.randint(20, 900))
    if r < 0.60:
        return "You try to %s %s, but miss!" % (v, m)
    if r < 0.64:
        return "You score a critical hit! (%d)" % rng.randint(100, 2000)
    if r < 0.68:
        return "%s has healed you for %d points." % (p, rng.randint(50, 900))
    if r < 0.72:
        return rng.choice([
            "You have slain %s!" % m,
            "You have been slain by %s!" % m,
            "%s has been slain by %s!" % (m, p),
        ])
    if r < 0.75:
        return "You gain %sexperience!!" % rng.choice(["", "party ", "raid "])
    if r < 0.755:
        return "You have gained a level! Welcome to level %d!" % rng.randint(2, 60)
    if r < 0.77:
        who = "You have" if rng.random() < 0.5 else "%s has" % p
        return "--%s looted a %s.--" % (who, rng.choice(ITEMS))
    if r < 0.78:
        return "You have entered %s." % rng.choice(ZONES)
    if r < 0.80:
        return "You begin casting %s." % rng.choice(SPELLS)
    if r < 0.83:
        return "%s %s, '%s'" % (p, rng.choice(
            ["tells you", "shouts", "says", "auctions", "tells the guild"]),
            rng.choice(CHATS))
    return rng.choice(NOISE)


def main():
    t = 1754514873
    out = []
    for i in range(N):
        if rng.random() < 0.35:
            t += rng.randint(0, 2)
        out.append("%s %s" % (stamp(t), body()))
        if rng.random() < 0.004:
            out.append("")                       # blank line
        if rng.random() < 0.004:
            out.append("  continued text with no header")
    sys.stdout.write("\n".join(out) + "\n")


if __name__ == "__main__":
    main()
