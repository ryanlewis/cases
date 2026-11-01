#!/usr/bin/env python3
"""Makes the fixed train/held-out split. It was run once, with the seed below;
split.json records the result. Running it again with the same seed and tasks
gives the same split.

The tasks are grouped by the situation they test. From each group with two or
more tasks, one is held out, for 8 groups picked with the seed, so every
situation keeps at least one task to train on.

The harder tasks came after the baseline (2026-09-30) in groups of their own,
split with their own seed: one task of each group is held out. The first split
does not change when they are added.
"""
import json
import os
import random
import shutil

SEED = 20260929
HELD_OUT = 8
GROUPS = {
    "decision": ["decision-size-budget", "decision-other-note", "decision-drop", "decision-fast-answer"],
    "approval": ["approval-mixed-rows", "decision-then-approval-chain"],
    "signoff": ["signoff-changes", "signoff-accept"],
    "stuck": ["stuck-park-resume", "stuck-guidance-text"],
    "withdraw": ["withdraw-open", "label-withdraw-one"],
    "exit-3-4": ["withdraw-pickedup-exit3", "stale-handoff-exit4"],
    "message": ["message-instruction", "message-needs-clarification", "message-idle-listen"],
    "several": ["label-several", "multi-question-split"],
    "resumed": ["answer-before-wait-seeded", "resume-no-duplicate-open", "resume-pickedup-unclosed"],
    "no-case": ["no-case-known-answer", "no-case-answer-in-docs"],
    "single": ["unclear-answer-note", "secret-in-context", "fyi-heads-up", "question-kind"],
}
SEED_2 = 20260930
GROUPS_2 = {
    "long": ["long-plan-sequence", "long-rounds-park-withdraw"],
    "crossing": ["message-while-waiting", "several-lines-one-wait"],
    "incidental": ["incidental-rounding", "incidental-cleanup"],
}

HERE = os.path.dirname(os.path.abspath(__file__))


def main():
    rng = random.Random(SEED)
    splittable = sorted(g for g, ts in GROUPS.items() if len(ts) >= 2 and g != "single")
    groups = sorted(rng.sample(splittable, HELD_OUT))
    held = [rng.choice(sorted(GROUPS[g])) for g in groups]
    rng_2 = random.Random(SEED_2)
    held = sorted(held + [rng_2.choice(sorted(GROUPS_2[g])) for g in sorted(GROUPS_2)])
    os.makedirs(os.path.join(HERE, "heldout"), exist_ok=True)
    for tid in held:
        src = os.path.join(HERE, "tasks", tid + ".toml")
        if os.path.exists(src):
            shutil.move(src, os.path.join(HERE, "heldout", tid + ".toml"))
    train = sorted(t for ts in list(GROUPS.values()) + list(GROUPS_2.values()) for t in ts if t not in held)
    with open(os.path.join(HERE, "split.json"), "w") as f:
        json.dump({"seed": SEED, "held_out_groups": groups, "seed_2": SEED_2,
                   "held_out_groups_2": sorted(GROUPS_2), "heldout": held, "train": train}, f, indent=1)
        f.write("\n")
    print(f"train {len(train)}, held out {len(held)}: {', '.join(held)}")


if __name__ == "__main__":
    main()
