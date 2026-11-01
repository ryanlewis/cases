# Offline audit of real sessions

Grades real Claude Code sessions (`~/.claude/projects/*/*.jsonl`) against a skill's checkable rules, with no model call. It only reads transcripts and writes one report. The report holds counts, rule names and session ids, never transcript text; it is built from private transcripts, so it is not committed.

```sh
python3 evals/audit/audit.py --rules evals/audit/rules_cases.py --rules evals/audit/rules_team_lead.py \
    --out ~/dev/notes/projects/cases/skill-audit-<date>.md
```

- `history.py` reads a real transcript into the eval harness's Transcript: it drops subagent (sidechain) turns, ends background tasks where their `<task-notification>` arrived, takes a background wait's output from a later read of its file when the file is gone, numbers the human turns, and treats heredoc bodies and quoted strings as data, not commands.
- A rules module gives `NAME`, `select(session)`, and `RULES` as (name, description, check), where a check returns (passes, failures) or None. It may add `SPLIT` (label, ISO time) to split sessions before and after a change, `SKIPPED` and `JUDGEMENT` (rules it does not grade, and why), and `cost(session)`.
- `rules_cases.py` reuses the eval's graders (`evals/cases-skill/graders.py`) for every rule a transcript alone settles, splits at #78, and counts cost per decision as the cost study did. `cases` calls on another store (`--store`, `CASES_STORE=`) are CLI testing and are dropped.
- `rules_team_lead.py` checks the team-lead skill's mechanical rules (notify_when_idle, the Reporting block, agent start flags and names, /effort on `built`, /code-review levels, checks before merges, case addressing, one label wait) and lists the ones that need judgement. The skill lives in the vault and changes often; the report dates each rule's first pass as the only hint of when it arrived.
- The eval's own sessions (`cases-eval-*`) and scratch probes are left out.

`python3 evals/audit/test_audit.py` tests it on hand-built session files.
