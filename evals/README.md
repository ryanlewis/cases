# Evals

Evals of the agent skills against real Claude Code sessions. They cost money and need a logged-in `claude`, so they are not part of CI.

- `harness/`: the pieces a suite reuses. `runner.py` runs one task (a fresh workspace and case store, a Claude Code session, a scripted human), `transcript.py` reads a session's stream-json transcript, `judge.py` asks a small model checkable claims in batches, and `stats.py` has the Wilson interval.
- `cases-skill/`: the suite for the `cases` skill (`crates/cli/skill/SKILL.md`). See its [README](cases-skill/README.md).

Python 3.11 or later, standard library only.
