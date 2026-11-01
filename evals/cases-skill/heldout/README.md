# Held out

These 11 tasks are the test set (seeds 20260929 and 20260930, see `../split.json`). A hillclimb on the skill must not read them: it improves against `--split train` and runs `--split heldout` only as its final check.
