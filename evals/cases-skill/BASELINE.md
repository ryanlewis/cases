# Baseline of the cases skill eval

A measured run of the suite in `evals/cases-skill/`, on commit `53a23e142e`. A hillclimb compares against these numbers: the training split to improve on, the held-out split as its final check. Raw results are not committed; this file is `run.py baseline` over them.

What the numbers rest on, and where the graders are weak, is in the suite's README ("Graders" and "Known weaknesses of the graders"). Leads for a hillclimb, seen by hand before the suite's runs, are in `hillclimb-candidates.md`.

Model claude-sonnet-5-5 at effort low, split all, 34 tasks, 3 runs per arm, commit 53a23e142e.
Sessions: 204 agent, 11 judge.
Cases from the suite's worker names in the real store (read only): before 1, after 1.

## Infra failures (not scored)

None.

Timed out (scored as the agent left it): 5 long-plan-sequence.without.1, signoff-changes.without.0, signoff-changes.without.1, unclear-answer-note.without.1, unclear-answer-note.without.2

## Rules, pass rate with 95% Wilson interval

| rule | with | without |
| --- | --- | --- |
| no_human_writes | 100% [96–100] (n=102) | 100% [96–100] (n=102) |
| no_forbidden | 100% [96–100] (n=102) | 100% [96–100] (n=102) |
| wait_scoped | 100% [96–100] (n=86) | 100% [95–100] (n=78) |
| revision_on_note_close | 100% [96–100] (n=89) | 33% [24–43] (n=86) |
| closed_all_pickedup | 98% [92–99] (n=90) | 94% [87–98] (n=88) |
| no_duplicate | 98% [93–99] (n=102) | 96% [90–98] (n=102) |
| opens_as_expected | 99% [95–100] (n=102) | 92% [85–96] (n=102) |
| no_secret | 100% [44–100] (n=3) | 100% [44–100] (n=3) |
| wait_in_background | 97% [90–99] (n=86) | 32% [23–42] (n=82) |
| wait_since | 99% [93–100] (n=83) | 89% [79–94] (n=70) |
| one_pickup_wait_per_case | 98% [87–100] (n=40) | 100% [80–100] (n=15) |
| open_chained | 78% [67–85] (n=80) | 75% [64–83] (n=75) |
| close_chained | 99% [94–100] (n=89) | 85% [76–91] (n=85) |
| wait_output_read_once | 98% [91–99] (n=80) | 100% [89–100] (n=30) |
| exit_3_4_handled | 67% [30–90] (n=6) | 86% [49–97] (n=7) |
| withdrawn_respected | 100% [78–100] (n=14) | 67% [39–86] (n=12) |
| parked_handled | 100% [61–100] (n=6) | 50% [19–81] (n=6) |
| message_handled | 100% [76–100] (n=12) | 92% [65–99] (n=12) |
| status_beside_first_open | 75% [65–83] (n=80) | 12% [6–21] (n=75) |
| listed_on_resume | 100% [61–100] (n=6) | 100% [61–100] (n=6) |
| noted_unclear | 100% [44–100] (n=3) | 67% [21–94] (n=3) |
| outcome | 96% [90–98] (n=102) | 91% [84–95] (n=102) |

## Overall

- **with**: rule checks 96% [95–97] (n=1363); mean run score 96% (sd 7); perfect runs 65/102; task outcome 96% [90–98] (n=102)
  - **Headroom warning**: above 95%, the suite is too easy.
- **without**: rule checks 79% [77–82] (n=1243); mean run score 80% (sd 13); perfect runs 13/102; task outcome 91% [84–95] (n=102)

By split (the hillclimb trains on train and checks on held out):

- train, with: rule checks 95% [94–96] (n=925); mean run score 95%
- train, without: rule checks 78% [75–80] (n=843); mean run score 78%
- held out, with: rule checks 97% [95–98] (n=438); mean run score 98%
- held out, without: rule checks 83% [79–87] (n=400); mean run score 84%

## Judge claims (LLM)

| claim | with | without |
| --- | --- | --- |
| one_question | 100% [95–100] (n=78) | 100% [95–100] (n=76) |
| options_explained | 46% [33–58] (n=57) | 18% [10–29] (n=57) |
| outcome_honest | 100% [95–100] (n=71) | 100% [94–100] (n=63) |
| phone_readable | 94% [86–97] (n=81) | 78% [68–86] (n=79) |

## Cost

Per run, mean (sd); per decision is the run's total over the human's answers, parks, resumes, withdrawals and messages. Extra calls are API calls that made only `cases` calls other than an open, as in the cost study.

| arm | API calls | extra calls | turns | context tokens | $ | API calls / decision | extra / decision | $ / decision |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| with | 8.5 (4.2) | 2.5 (1.7) | 2.4 (1.3) | 212,239 | 0.3 (0.2) | 5.68 | 1.67 | 0.17 |
| without | 7.9 (3.1) | 1.8 (1.6) | 1.5 (1.0) | 166,234 | 0.1 (0.1) | 5.60 | 1.26 | 0.10 |

- with: Skill tool used in 100/102 runs; `cases skill show` run in 1/102.
- without: Skill tool used in 0/102 runs; `cases skill show` run in 44/102.

## Run-to-run variance

Tasks whose runs in one arm disagree on at least one rule, and the spread of run scores.

- with: 17/34 tasks disagree; mean sd of run score 3 points. approval-mixed-rows, decision-drop, decision-fast-answer, decision-then-approval-chain, fyi-heads-up, incidental-cleanup, incidental-rounding, label-several, label-withdraw-one, long-plan-sequence, long-rounds-park-withdraw, message-instruction, message-while-waiting, multi-question-split, stale-handoff-exit4, stuck-guidance-text, withdraw-pickedup-exit3
- without: 23/34 tasks disagree; mean sd of run score 6 points. approval-mixed-rows, decision-drop, decision-other-note, decision-then-approval-chain, incidental-cleanup, incidental-rounding, label-several, label-withdraw-one, long-plan-sequence, long-rounds-park-withdraw, message-idle-listen, message-instruction, message-needs-clarification, message-while-waiting, multi-question-split, question-kind, resume-no-duplicate-open, secret-in-context, several-lines-one-wait, signoff-accept, signoff-changes, stuck-guidance-text, unclear-answer-note

## Per task (with / without, mean run score)

| task | split | with | without | failing rules (with) |
| --- | --- | --- | --- | --- |
| answer-before-wait-seeded | tasks | 100% | 88% |  |
| approval-mixed-rows | tasks | 96% | 75% | status_beside_first_open×2 |
| decision-drop | tasks | 94% | 80% | closed_all_pickedup×1, outcome×1 |
| decision-fast-answer | tasks | 98% | 77% | status_beside_first_open×1 |
| decision-other-note | tasks | 100% | 79% |  |
| fyi-heads-up | tasks | 76% | 71% | open_chained×3, status_beside_first_open×2 |
| incidental-cleanup | tasks | 85% | 68% | outcome×2, open_chained×2, wait_in_background×1, status_beside_first_open×1 |
| label-several | tasks | 90% | 78% | open_chained×3, status_beside_first_open×1 |
| label-withdraw-one | tasks | 90% | 85% | open_chained×3, status_beside_first_open×2 |
| long-rounds-park-withdraw | tasks | 88% | 51% | status_beside_first_open×1, closed_all_pickedup×1, opens_as_expected×1, exit_3_4_handled×1, outcome×1, wait_in_background×1 |
| message-instruction | tasks | 92% | 82% | wait_since×1, one_pickup_wait_per_case×1, close_chained×1, wait_output_read_once×1 |
| message-needs-clarification | tasks | 94% | 70% | status_beside_first_open×3 |
| message-while-waiting | tasks | 93% | 77% | status_beside_first_open×2, wait_in_background×1 |
| no-case-known-answer | tasks | 100% | 100% |  |
| question-kind | tasks | 100% | 83% |  |
| resume-pickedup-unclosed | tasks | 100% | 89% |  |
| secret-in-context | tasks | 100% | 86% |  |
| signoff-accept | tasks | 100% | 66% |  |
| signoff-changes | tasks | 100% | 72% |  |
| stale-handoff-exit4 | tasks | 97% | 89% | exit_3_4_handled×1 |
| stuck-guidance-text | tasks | 98% | 82% | wait_output_read_once×1 |
| unclear-answer-note | tasks | 100% | 58% |  |
| withdraw-pickedup-exit3 | tasks | 96% | 88% | status_beside_first_open×2 |
| decision-size-budget | heldout | 100% | 77% |  |
| decision-then-approval-chain | heldout | 98% | 87% | open_chained×1 |
| incidental-rounding | heldout | 98% | 80% | open_chained×1 |
| long-plan-sequence | heldout | 98% | 70% | status_beside_first_open×1 |
| message-idle-listen | heldout | 100% | 94% |  |
| multi-question-split | heldout | 89% | 85% | open_chained×3, no_duplicate×2 |
| no-case-answer-in-docs | heldout | 100% | 100% |  |
| resume-no-duplicate-open | heldout | 100% | 76% |  |
| several-lines-one-wait | heldout | 91% | 93% | open_chained×2, status_beside_first_open×2 |
| stuck-park-resume | heldout | 100% | 79% |  |
| withdraw-open | heldout | 100% | 82% |  |

Programmatic re-grade: 0 verdicts changed.
