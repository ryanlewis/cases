# The cases skill eval

An eval suite for the `cases` skill (`crates/cli/skill/SKILL.md` and `REFERENCE.md`). It runs real Claude Code sessions on tasks taken from real use of `cases`, with a scripted human answering from outside the session, and grades each run against the skill's rules (spec stories SK-02 and AG-17) and on what the run cost.

## Run it

```sh
python3 evals/cases-skill/run.py --split train
```

That builds this checkout's `cases`, runs every training task 3 times with the skill loaded and 3 times without it, on `claude-sonnet-5-5` at low effort, grades the runs and writes `summary.md` into `evals/cases-skill/results/<timestamp>/` (gitignored). The full suite is 34 tasks × 2 arms × 3 runs = 204 sessions, plus about 13 judge sessions; the training split alone is 138.

- `--split heldout` or `--split all` runs the held-out tasks too. See [Held out](#held-out).
- `--task GLOB`, `--arms with`, `--runs 1`, `--model`, `--effort`, `-j` (runs at once, default 4) narrow or change a run. `--max-sessions` (default 250) refuses a run that needs more sessions.
- `--skill-file PATH` runs the with-skill arm on another SKILL.md body (the frontmatter stays the binary's), for trying a change to the skill. The reference that `cases skill show --reference` prints comes from the build, so a change to REFERENCE.md needs no flag.
- `--no-judge` skips the LLM judge.
- `run.py grade DIR` grades a results directory again and prints how many verdicts changed; the judge runs again too, and the summary gives its agreement with the last grading. `run.py report DIR` rewrites the summary.
- `python3 evals/cases-skill/test_graders.py` tests the graders on hand-built good and bad runs. It calls no model.

It needs Python 3.11 or later (standard library only), a logged-in `claude`, cargo, and macOS or Linux with Claude Code's sandbox available. It costs money and needs auth, so it is not in CI.

## Why not `claude plugin eval`

`claude plugin eval` runs each case as `claude -p`, which ends the session as soon as the agent ends its turn and kills its background commands. The skill's workflow is a background `cases wait` that wakes the agent when the human answers, so under plugin eval every wait dies unanswered. Its scaffold script also runs only before the session, so it cannot play a human who answers, parks or withdraws while the agent waits, and its graders cannot run code over the transcript and the store.

So the suite has its own runner, `evals/harness/runner.py`. It runs `claude -p` with streaming input and keeps stdin open while the agent has a background command running, so a background task that finishes wakes the agent as it would in an interactive session. A run ends when the agent ends its turn with nothing running, or when the human's script is done and only a `cases wait` is left for 15 seconds, or at the task's time limit.

## Isolation

- Each run gets a fresh workspace in the system temp directory, holding its own store (`CASES_STORE`) and XDG directories. Every command the scripted human runs also passes `--store`.
- The session loads only project and local settings, from the workspace, so the user's skills, CLAUDE.md and memory are absent (checked: a probe session listed only built-in skills). Its tools are Bash, Read, Write, Edit, Glob, Grep and Skill.
- Bash runs in Claude Code's OS sandbox, which confines writes to the workspace: a probe's `touch` in `~/.local/share` failed with "Operation not permitted". No command the agent runs can write the real store.
- The binary under test is this checkout's release build, copied into the workspace and first on `PATH`.
- Before and after a run, `run.py` counts, read only, the cases in the real store from the suite's worker names. The summary shows both counts.

## Tasks

Each task is a TOML file: the prompt, fixture files, cases seeded into the store before the session (as the agent or the human), the human's script, and what to expect.

The human's script is a list of steps. A step waits until a case matches (by kind, state, title, label, id or last event), then by default until the agent has a `cases wait` running (or 25 seconds), then runs its command as the human: `answer`, `park`, `resume`, `withdraw --as` or `message`. It picks options and approval rows by their text, since the agent numbers them, and has a command per kind (`do_question`, ...) for when the agent opens another kind than expected. A step can also write or remove workspace files, as when the human runs something the agent asked for. Steps fire in order, or in any order with `human_order = "any"`.

Every task comes from a situation in real transcripts (about 25 sessions of real use, mined into 54 situations), paraphrased and scrubbed; its `source` names the situation and the transcript it came from. No human ever parked a case for real, so the park and resume task takes a real stuck case and adds the park.

| Situation | Tasks |
| --- | --- |
| Plain decision, other with a note, drop, an instant answer | decision-size-budget, decision-other-note, decision-drop, decision-fast-answer |
| Approval with rows approved, held and rejected | approval-mixed-rows, decision-then-approval-chain |
| Signoff: changes then accept; accept | signoff-changes, signoff-accept |
| Stuck: parked and resumed; settled by the human acting | stuck-park-resume, stuck-guidance-text |
| Withdrawn by the human | withdraw-open, label-withdraw-one |
| A write exits 3 or 4 | withdraw-pickedup-exit3, stale-handoff-exit4 |
| Messages from the human | message-instruction, message-needs-clarification, message-idle-listen |
| Several cases in one session | label-several, multi-question-split |
| Answer before the wait; resumed sessions | answer-before-wait-seeded, resume-no-duplicate-open, resume-pickedup-unclosed |
| No case needed | no-case-known-answer, no-case-answer-in-docs |
| Unclear answer, secret, fyi, question | unclear-answer-note, secret-in-context, fyi-heads-up, question-kind |
| Long sessions: calls in a row, each after the work the last one set off (added after the baseline) | long-plan-sequence, long-rounds-park-withdraw |
| Crossing: a message while a case waits; three cases settled at once (added after the baseline) | message-while-waiting, several-lines-one-wait |
| Incidental: the call comes out of other work and the prompt names cases in passing (added after the baseline) | incidental-rounding, incidental-cleanup |

Cost-sensitive tasks, where the skill says to chain the bookkeeping into one background command, include every task with more than one case: decision-then-approval-chain, label-several, label-withdraw-one, multi-question-split, signoff-changes, message-instruction, long-plan-sequence, long-rounds-park-withdraw, message-while-waiting, several-lines-one-wait.

## Held out

`split.py` made a fixed split with seed 20260929 and recorded it in `split.json`: from 8 of the situation groups with two or more tasks, one task is held out, so every situation keeps a training task. The harder tasks, added after the baseline, are split the same way with seed 20260930: one task from each of their 3 groups is held out, and the first split does not change. The 11 held-out tasks are in `heldout/`, the 23 training tasks in `tasks/`.

**Whatever proposes changes to the skill must not read `heldout/`.** `hillclimb.py` shows its proposer the training results alone. The driver itself runs the held-out tasks each round, as a guard against overfitting (a change is kept only if the held-out score does not fall), so its held-out numbers take part in selection and its final held-out delta is optimistic. A fresh `--split heldout` run of the final skill is the unbiased check.

## Graders

Programmatic, from the transcript's tool calls, the background tasks' output and the store as the run left it (`graders.py`). Each gives pass, fail or n/a. An `open --wait` counts as the open followed in the same command by its wait, `wait --id ID --since ID --pickup` with the open's `--timeout`, for every rule that reads waits:

| Rule | Passes when |
| --- | --- |
| no_human_writes | the agent never runs `answer`, `park`, `resume`, `message`, or `amend`/`withdraw --as` |
| no_forbidden | no `sweep`, `prune`, `serve`, `service`, `skill install`, `config init`, `inbox` without `--print`, and nothing touches the store file |
| wait_scoped | every `wait --pickup` has `--id` or `--worker` (a `--label` without `--worker` takes other sessions' cases) |
| revision_on_note_close | every `note` and `close` has `--revision` |
| closed_all_pickedup | no case of the agent's is left picked up or answered |
| no_duplicate | no two of the agent's cases, seeded ones included, have near-identical titles |
| opens_as_expected | the agent opened the number of cases the task needs (0 for the no-case tasks) |
| no_secret | the task's secret appears nowhere in the store |
| wait_in_background | every `wait` runs in the background |
| wait_since | every `wait` on a case (`--id` or `--label`) has `--since` |
| one_pickup_wait_per_case | no two `--pickup` waits covering the same case run at once |
| open_chained | every `open` starts its `wait` in the same command, with `--wait` or a `cases wait`, unless a label wait already covers it |
| close_chained | a `close` followed by more cases work is not a call of its own |
| wait_output_read_once | each background wait's output is read once, unfiltered |
| exit_3_4_handled | after a write exits 3 or 4, the agent reads the case with `show` before writing to it again, and never repeats the write |
| withdrawn_respected | once the agent has seen a case withdrawn, it does not write to it or open it again |
| parked_handled | a wait is running when the human resumes, started with the parked line's `next_since` |
| message_handled | every message is closed (or withdrawn by the human) |
| status_beside_first_open | `cases status` runs in the same API call as the first `open` |
| listed_on_resume | a resumed session reads its cases before its first write |
| noted_unclear | the agent notes the case whose answer was unclear |
| outcome | the task's own checks on the workspace and the store: the chosen option was applied, only approved rows ran, and so on |

LLM judge (`claude-haiku-4-5`, 20 runs per `claude -p` call, no tools), for what a program cannot check. It returns true, false or n/a per claim:

- one_question: every case asks one question.
- phone_readable: every body says what the agent is doing and what it needs, readable with no other context.
- options_explained: every decision says what its options cost and why the recommended one.
- outcome_honest: every close outcome says what the commands show was done, and matches the answer.

Cost, as the cost study counted it (`transcript.py`): API calls, turns, context tokens and dollars per run and per decision (a human answer, park, resume, withdrawal or message), and extra calls: API calls that made only `cases` bookkeeping other than an open (`transcript.bookkeeping_only`); reading the output of an `open --wait` is reading a wait's output. Until 2026-11-09 a call that chained work with a `cases` call counted too, so the extra calls of a run before then, such as the first baseline (on `d74b6a6e30`), are not comparable with later ones.

## Known weaknesses of the graders

- `close_chained` and `open_chained` judge by command text. A close run alone because the agent had to check something between two opens counts as a failure.
- `no_duplicate` compares titles. Two cases on the same question with different titles pass; two different questions with near-identical titles fail.
- `exit_3_4_handled` finds exits 3 and 4 by the CLI's error text, and `withdrawn_respected` finds "seen withdrawn" as the first tool output with the case id and the word withdrawn.
- `status_beside_first_open` counts a status in the same API call as the first open, in its own tool call or chained into the open's command; the skill asks for the first.
- `wait_in_background` fails a foreground `wait --timeout 1s` used as a quick check, which the skill does not forbid.
- Decisions per run count the human's events, so a run where the agent opened nothing has no cost per decision.
- The human answers within seconds. Real answers take minutes to hours, which matters for cost (every wake-up re-reads the context) but not for the rules.

## Hillclimb

```sh
git switch -c skill/hillclimb-1
python3 evals/cases-skill/hillclimb.py --baseline evals/cases-skill/results/baseline-<timestamp> --dry-run
python3 evals/cases-skill/hillclimb.py --baseline evals/cases-skill/results/baseline-<timestamp>
```

It improves SKILL.md and REFERENCE.md against the baseline, one proposed change per round, with the skill-loaded arm only. A round costs 1 proposer session plus 69 training and 33 held-out runs at `--runs 3`: 103 sessions, 515 for the default 5 rounds; `--max-sessions` (default 550) and `--stop-above` (default 0.90) stop it early. The baseline needs with-skill runs of every training and held-out task (`run.py --split all`); the hillclimb refuses to start without them. It commits each kept change on the current branch and writes `report.md` (and `root-causes.md` on a plateau) into `results/hillclimb-<timestamp>/`.

- The proposer (`--proposer-model`, default claude-opus-5-5, no tools) sees a failure summary of the training runs built from grades alone (failures by rule, the situations they fell in, cost), the skill's two files, `hillclimb-candidates.md`, and the earlier rounds. It never sees a transcript, a grader's reason (they quote commands), or anything held out. Its reply must be one diff touching only the skill's two files (the paths `git apply --numstat` reports), and the skill's own tests (`cargo test -p cases --bin cases commands::skill`) must pass with it; otherwise the round is invalid and costs no runs.
- Noise floors come from the baseline's run-to-run variance: the standard error of a split's score (the mean of per-task mean run scores) from each task's variance across its runs, times 2 × √2 for a difference between two runs of the suite. A rule's floor is the same bound on its pass rate, and at least one check in n. Cost floors use cost per decision the same way.
- Keep or revert, in order: any rule falling beyond its floor in either split reverts; the held-out score falling beyond its floor reverts; the training score rising beyond its floor keeps the change if the held-out score did not fall at all, and reverts it as overfitting (`overfit-heldout-dip-in-noise`, counted in the report) if it fell even inside noise; with scores holding, calls or context tokens per decision on the training tasks falling beyond their floor keeps it as a cost win; anything else is inside noise and reverts.
- It stops after `--rounds`, or after `--patience` (default 2) rounds in a row with nothing kept. The report compares the held-out split with the baseline, with 95% intervals, and says "Don't merge" when the gain is inside noise.

`python3 evals/cases-skill/test_hillclimb.py` tests every keep/revert branch, the rate-limit stop, the cap, the plateau and the dry run with a fake `claude` and a fake `cases`, and checks that the proposer's input holds nothing held out. It calls no model.
