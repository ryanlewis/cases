# Baseline of the cases skill eval

The first measured run of the suite in `evals/cases-skill/`, on commit `d74b6a6e30`, before any change to the skill. A hillclimb compares against these numbers: the training split to improve on, the held-out split as its final check. Raw results are not committed; this file is `run.py baseline` over them.

What the numbers rest on, and where the graders are weak, is in the suite's README ("Graders" and "Known weaknesses of the graders"). Hillclimb candidate #1, seen before this run: SKILL.md says to scope every `wait --pickup` with `--id` or with `--label` and `--worker`, but its messages section describes a listening wait with `--worker` alone. The grader accepts `--worker` alone.

Model claude-sonnet-5-5 at effort low, split all, 34 tasks, 3 runs per arm, commit d74b6a6e30.
Sessions: 204 agent, 13 judge.
Cases from the suite's worker names in the real store (read only): before 0, after 0.

## Infra failures (not scored)

None.

Timed out (scored as the agent left it): 8 decision-fast-answer.without.0, decision-fast-answer.without.1, decision-fast-answer.without.2, long-plan-sequence.without.0, long-plan-sequence.without.2, message-while-waiting.without.1, signoff-changes.without.2, stuck-park-resume.without.0

## Rules, pass rate with 95% Wilson interval

| rule | with | without |
| --- | --- | --- |
| no_human_writes | 100% [96–100] (n=102) | 100% [96–100] (n=102) |
| no_forbidden | 100% [96–100] (n=102) | 100% [96–100] (n=102) |
| wait_scoped | 100% [96–100] (n=86) | 100% [95–100] (n=72) |
| revision_on_note_close | 100% [96–100] (n=90) | 17% [10–27] (n=77) |
| closed_all_pickedup | 100% [96–100] (n=90) | 92% [85–97] (n=80) |
| no_duplicate | 100% [96–100] (n=102) | 96% [90–98] (n=102) |
| opens_as_expected | 100% [96–100] (n=102) | 99% [95–100] (n=102) |
| no_secret | 100% [44–100] (n=3) | 100% [44–100] (n=3) |
| wait_in_background | 90% [81–94] (n=86) | 11% [6–20] (n=79) |
| wait_since | 99% [93–100] (n=83) | 16% [9–27] (n=68) |
| one_pickup_wait_per_case | 100% [91–100] (n=39) | 100% [57–100] (n=5) |
| open_chained | 64% [53–74] (n=81) | 3% [1–9] (n=77) |
| close_chained | 99% [94–100] (n=90) | 93% [85–97] (n=75) |
| wait_output_read_once | 100% [95–100] (n=79) | 100% [70–100] (n=9) |
| exit_3_4_handled | 100% [61–100] (n=6) | 78% [45–94] (n=9) |
| withdrawn_respected | 100% [80–100] (n=15) | 93% [69–99] (n=14) |
| parked_handled | 100% [61–100] (n=6) | 50% [19–81] (n=6) |
| message_handled | 100% [76–100] (n=12) | 75% [47–91] (n=12) |
| status_beside_first_open | 88% [79–93] (n=81) | 8% [4–16] (n=77) |
| listed_on_resume | 100% [61–100] (n=6) | 100% [61–100] (n=6) |
| noted_unclear | 100% [44–100] (n=3) | 100% [44–100] (n=3) |
| outcome | 98% [93–99] (n=102) | 81% [73–88] (n=102) |

## Overall

- **with**: rule checks 96% [95–97] (n=1366); mean run score 96% (sd 6); perfect runs 61/102; task outcome 98% [93–99] (n=102)
  - **Headroom warning**: above 95%, the suite is too easy.
- **without**: rule checks 68% [65–70] (n=1182); mean run score 69% (sd 16); perfect runs 10/102; task outcome 81% [73–88] (n=102)

By split (the hillclimb trains on train and checks on held out):

- train, with: rule checks 95% [94–97] (n=925); mean run score 95%
- train, without: rule checks 69% [65–72] (n=817); mean run score 69%
- held out, with: rule checks 98% [96–99] (n=441); mean run score 98%
- held out, without: rule checks 66% [61–71] (n=365); mean run score 69%

## Judge claims (LLM)

| claim | with | without |
| --- | --- | --- |
| one_question | 96% [88–99] (n=70) | 96% [88–99] (n=72) |
| options_explained | 67% [53–79] (n=46) | 17% [9–30] (n=48) |
| outcome_honest | 100% [94–100] (n=66) | 96% [87–99] (n=53) |
| phone_readable | 96% [88–99] (n=70) | 62% [51–73] (n=72) |

Judge agreement with the previous grading: 142/160 claim verdicts the same

## Cost

Per run, mean (sd); per decision is the run's total over the human's answers, parks, resumes, withdrawals and messages. Extra calls are API calls that made only `cases` calls other than an open, as in the cost study.

| arm | API calls | extra calls | turns | context tokens | $ | API calls / decision | extra / decision | $ / decision |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| with | 8.8 (4.5) | 3.8 (2.2) | 2.5 (1.5) | 167,336 | 0.2 (0.2) | 5.62 | 2.54 | 0.14 |
| without | 7.8 (2.9) | 2.7 (2.0) | 1.1 (0.7) | 124,608 | 0.1 (0.1) | 5.46 | 1.89 | 0.06 |

- with: Skill tool used in 96/102 runs; `cases skill show` run in 0/102.
- without: Skill tool used in 0/102 runs; `cases skill show` run in 12/102.

## Run-to-run variance

Tasks whose runs in one arm disagree on at least one rule, and the spread of run scores.

- with: 19/34 tasks disagree; mean sd of run score 2 points. approval-mixed-rows, decision-fast-answer, decision-size-budget, decision-then-approval-chain, fyi-heads-up, incidental-cleanup, incidental-rounding, label-withdraw-one, long-rounds-park-withdraw, message-needs-clarification, message-while-waiting, multi-question-split, resume-no-duplicate-open, secret-in-context, signoff-accept, signoff-changes, stuck-park-resume, unclear-answer-note, withdraw-pickedup-exit3
- without: 16/34 tasks disagree; mean sd of run score 5 points. approval-mixed-rows, label-withdraw-one, long-plan-sequence, long-rounds-park-withdraw, message-idle-listen, message-instruction, message-needs-clarification, message-while-waiting, multi-question-split, secret-in-context, several-lines-one-wait, signoff-accept, signoff-changes, stale-handoff-exit4, stuck-park-resume, withdraw-pickedup-exit3

## Per task (with / without, mean run score)

| task | split | with | without | failing rules (with) |
| --- | --- | --- | --- | --- |
| answer-before-wait-seeded | tasks | 97% | 86% | wait_in_background×1 |
| approval-mixed-rows | tasks | 98% | 82% | outcome×1 |
| decision-drop | tasks | 100% | 62% |  |
| decision-fast-answer | tasks | 98% | 48% | open_chained×1 |
| decision-other-note | tasks | 100% | 62% |  |
| fyi-heads-up | tasks | 76% | 71% | open_chained×3, status_beside_first_open×2 |
| incidental-cleanup | tasks | 85% | 57% | wait_in_background×3, open_chained×2, outcome×1 |
| label-several | tasks | 100% | 65% |  |
| label-withdraw-one | tasks | 96% | 94% | open_chained×1, status_beside_first_open×1 |
| long-rounds-park-withdraw | tasks | 96% | 47% | open_chained×1, close_chained×1 |
| message-instruction | tasks | 94% | 83% | open_chained×3 |
| message-needs-clarification | tasks | 91% | 58% | status_beside_first_open×3, wait_in_background×1 |
| message-while-waiting | tasks | 84% | 57% | wait_in_background×3, status_beside_first_open×3, wait_since×1 |
| no-case-known-answer | tasks | 100% | 100% |  |
| question-kind | tasks | 100% | 62% |  |
| resume-pickedup-unclosed | tasks | 100% | 89% |  |
| secret-in-context | tasks | 98% | 73% | open_chained×1 |
| signoff-accept | tasks | 95% | 56% | open_chained×2 |
| signoff-changes | tasks | 96% | 54% | open_chained×2 |
| stale-handoff-exit4 | tasks | 100% | 93% |  |
| stuck-guidance-text | tasks | 93% | 62% | open_chained×3 |
| unclear-answer-note | tasks | 98% | 64% | open_chained×1 |
| withdraw-pickedup-exit3 | tasks | 98% | 73% | status_beside_first_open×1 |
| decision-size-budget | heldout | 98% | 63% | open_chained×1 |
| decision-then-approval-chain | heldout | 98% | 63% | open_chained×1 |
| incidental-rounding | heldout | 98% | 61% | open_chained×1 |
| long-plan-sequence | heldout | 100% | 49% |  |
| message-idle-listen | heldout | 100% | 64% |  |
| multi-question-split | heldout | 98% | 58% | open_chained×1 |
| no-case-answer-in-docs | heldout | 100% | 100% |  |
| resume-no-duplicate-open | heldout | 97% | 83% | wait_in_background×1 |
| several-lines-one-wait | heldout | 93% | 90% | open_chained×3 |
| stuck-park-resume | heldout | 96% | 59% | open_chained×2 |
| withdraw-open | heldout | 100% | 64% |  |

Programmatic re-grade: 0 verdicts changed.
