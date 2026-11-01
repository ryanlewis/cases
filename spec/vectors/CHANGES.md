# Vector changes

This file lists every vector changed, added or removed since the set's first version, file by file, with the user story or the decision behind it. Story ids (AG-, HU-, ST-, CL-, IN-) name the user stories cases is built against. A case id names a decision in the decision log. A vector is named by its description; where a description changed, the old one follows "was".

## The rules

Each rule these changes brought in, and where the vectors carry it. A rule for the CLI or the web inbox has no vector: the vectors cover only the core.

| Rule | Vectors | Decided by |
| --- | --- | --- |
| Resume is human-only: an agent's `resume` is an `author` error | `transitions.json`; rows in `fold.json` and `writer.json` | HU-05, case `2026-09-24T19-42-22Z-agent-resume-drop-the-rule-as-well-as-the` |
| A stale write exits 4. The core's part, the `stale` category, is as it was | None: CLI | CL-01, case `2026-09-24T19-42-22Z-should-a-stale-write-get-its-own-exit-code` |
| The human writes `withdraw` as well as the agent. The agent withdraws only an `open` case; the human also an `answered` or `pickedup` one, which keeps its answer and pickup. Neither withdraws a `closed`, `withdrawn` or `parked` case | `transitions.json` | ST-14, case `2026-09-24T19-42-22Z-sweep-who-is-recorded-as-withdrawing`; AG-11; case `Q8PA4`: the human ends a case whose agent session died |
| Only an amend that sets a body other than the case's, or adds options or rows, moves `amend_seq`. Links, labels and context do not | `amend.json`, `answer.json` | ST-04, case `2026-09-24T19-42-22Z-which-amends-count-as-changing-the-question` |
| An answer carries `amend_seq`, the case's as its writer read it, 0 when left out. One below the case's is `stale`, checked before the state, on a write and on a replay. One above it is `invalid` | `answer.json`, `fold.json` | ST-04, ST-03; case `2026-09-24T20-38-01Z-st-04-error-for-an-answer-that-missed-the` |
| An answer's `amend_seq` is a count: one below 0 is `malformed`, not `stale` | `answer.json`, as an append and a replay | ST-04, case `2026-09-24T22-54-01Z-negative-amend-seq-on-an-answer-which-error` |
| JSON `-0` is a whole number, 0; `-0.0` has a fraction part and is `malformed` where a whole number goes | `fold.json` | ST-06; the review of the core crate (PR #2), which fixed the reader |
| `cases park ID` is its own command. Who may park, the human, and which cases, an open stuck case, are as they were | None: CLI | HU-03, case `2026-09-24T19-42-22Z-park-its-own-command` |
| The web status codes are part of the contract: 409 stale, 422 refused, 404 unknown case, 400 bad form | None: web | IN-04, case `2026-09-24T19-42-22Z-are-the-web-status-codes-part-of-the-contract` |
| A written open is refused for an option given twice, a link of only white space, a link given twice, or a `worker`, `brief` or `context` of only white space. A stored open with any of them still folds | `open.json`, each rule as an append and a replay | AG-02 |
| Field names match exactly: `Kind` is not `kind` | `fold.json` | ST-06 |
| A key given twice, at any depth of a record, is `malformed` | `fold.json` | ST-06 |
| A record of `null`, or a field of `null`, is `malformed`, not a record or field left out | `fold.json` | ST-06 |
| Times are read as RFC 3339's `date-time`: a lower-case `t` or `z`, and a time spelt with a JSON escape, are read; an hour of one digit, a comma before the fraction, an offset of `+24:00` or with a minute of 60, and a space in place of the `T` are `malformed`. A leap second stays `malformed` | `fold.json` | ST-06; case `2026-09-24T21-24-26Z-is-a-leap-second-23-59-60-read` for the leap second |
| A missing time is left out, and there is no zero time: `0001-01-01T00:00:00Z` is a time in year 1 | `fold.json`, `writer.json` | ST-06 |
| A field at its zero value (`""`, 0, `false`, `[]`) reads as a field left out. An empty `worker` is no worker, while one of only white space is refused on write | `answer.json`, as they were; `open.json` | Case `2026-09-24T20-38-25Z-does-a-zero-value-field-count-as-given`; AG-02 |
| A missing kind or urgency is `invalid`, as a missing title is. `unknown` is kept for a value this version does not know | `open.json` | AG-01, ST-05 |
| An appended open is checked in the order a stored one is: its actor before its kind | `open.json` | ST-02 |
| No record from before this codebase is read. Stores made by earlier versions of this codebase are read, and upgraded, as ever | `fold.json`: two vectors removed | Case `2026-09-24T19-31-55Z-switch-day-what-happens-to-your-existing-go`; ST-05, ST-15 |
| Fields from a newer version still fold, and stay in the event's `data` | `fold.json`, as they were | ST-05 |
| A close may say `expired`: the human's side closed a picked-up briefing no agent closed in time. Only a briefing closes expired, on a write and on a replay; it still needs an outcome, and is written under the agent's role | `transitions.json`, `fold.json` | AG-19, decided by Ryan on 1 Nov |
| An event's actor is only `{id}`: 22 base64url characters, 128 random bits, never derived from an email address or an account. `author` gives the role. Names and kinds live in the store's table of actors, local and hosted; a hosted store can delete a person's row, and the events then keep a bare id | `open.json`, `fold.json`, `writer.json` | Case `2026-09-24T21-30-51Z-actor-clean-slate-events-hold-only-id-names-in`; the identity research. ST-12, HU-06, AG-01, AG-13 and IN-02 change to match |
| `for` still names a person as text. It becomes an actor id before anything is hosted: a follow-up, not in this PR | None yet | Case `2026-09-24T21-30-51Z-actor-clean-slate-events-hold-only-id-names-in` |
| Case ids, and the title slug after them, belong to the store and the CLI | None: the vectors never had ids | ST-10, cases `2026-09-24T19-27-13Z-case-ids-which-opaque-format`, `2026-09-24T19-42-22Z-hash-ids-keep-a-title-slug-beside-them` |
| The `file` property of `events` and `problems` stays | As they were | AG-13, ST-05 |
| A decision's open may carry `recommend`, the option the agent recommends, numbered from 1 as a choice is; 0 or left out is none. It is `invalid` on any other kind and when it names no option (other is not one), on a replay as on a write. An amend may set it or change it, checked against the options as amended, but not take it away; the one the case has changes nothing. A new one does not move `amend_seq`: the recommend is advice about the question, not the question (ST-04), and a build from before the field skips an amend that sets only a recommend as changing nothing, so if a new recommend moved `amend_seq` the two builds would work out different ones from the same rows, and each would skip the other's answers as stale; the schema version (ST-07) does not stop the older build, since the tables are the same. An answer is never checked against it | `open.json`, `amend.json`, `answer.json`, `fold.json` | Case `AXs1i` (26 Sep): recommend is a field of the v1 contract, in place of a `Recommendation:` line in the body; ST-04, ST-07 |
| A `message` is the one kind the human opens: a message from the human to an agent. It starts `sent`, the agent's `pickup` makes it `pickedup`, and the agent's `close` makes it `closed`. The human may amend it while it is sent, and withdraw it while it is sent or picked up. It takes no answer, note, park or resume. It goes to a worker or a label: with neither, or with a worker of only white space and no label, it is `invalid`, on a replay as on a write | `transitions.json`, `open.json`, `amend.json`, `fold.json` | Cases `rvRCl`, `8g2R0` and `oPWlr`, in the discovery Ryan signed off as case `4teOl` (label `discovery-human-messages`, 27 Sep); the state `sent`, decided for Ryan by cases-sh on 27 Sep, in the design note `message-kind-design-2026-09-27` |
| `open` and `amend` belong to the side that opens the case's kind: the human's on a message, the agent's on every other kind. The other side's is `author`, checked once the kind is known: an open's straight after its kind, an amend's before the case's state. Either author passes the first check of the author of these two events, so their record is read first. A second open is a `transition` whoever writes it | `transitions.json`, `open.json`, `amend.json`, `fold.json` | Case `oPWlr`: only the human sends a message; the `author` category, decided for Ryan by cases-sh on 27 Sep, in the design note `message-kind-design-2026-09-27` |
| An open may carry `re`, the case it is about, as text. One of only white space is `invalid`, on a replay as on a write. The core never reads it as an id | `open.json` | Case `8g2R0`: the agent's questions go into ordinary cases linked to the message |
| An approval row may carry `by`, who carries the step out: left out for the agent, or `human`. Any other value, `agent` and `Human` among them, is `invalid`, on an open and an amend, on a replay as on a write. Verdicts are as they were: on the human's row, `approve` means the human has done it | `open.json`, `amend.json` | Case `i2lDf` (1 Oct): a step only the human can carry out is a field of the row, in place of a "no agent action" line in its script |
| A note that reopens an answered or picked-up case says so with `reopen: true`. The writer refuses one without it as a `transition`, checked with the state; a stored note without it still reopens the case, as every note did before the field, so stores written before it replay as they did. `reopen` on a note of an open case is `invalid`, on a replay as on a write | `transitions.json`, `fold.json`, `writer.json` | Cases `6M6gx` and `AV8s1` (1 Oct): agents reopened cases by accident with a follow-up note, and lost the answer |
| The agent's amend with `reopen: true` reopens an answered or picked-up case as a reopening note does: the case is open, the answer and pickup are cleared, and the events keep them. Without it an amend is open-only, as before. On a case that is open or sent it is `invalid`; a message is never reopened. An amend that only reopens changes nothing, and is `invalid` | `transitions.json`, `amend.json` | Cases `6M6gx` and `AV8s1` (1 Oct): after a signoff answered `changes`, the agent points the case at the new work and asks again, in place of closing it and opening another |
| An amend's `title` replaces the case's; one of only white space is `invalid`. `drop_links` takes links off the case: each one the case has, given once, and not added by the same amend, or the amend is `invalid`. Neither moves `amend_seq`, for the reason a recommend does not | `amend.json` | Case `AV8s1` (1 Oct); ST-04 |
| A `briefing` is an agent's kind that shows finished work. Its body is required. `landed` and `left_out` are its only, and options, a recommend and rows are not: on the wrong kind each is `invalid`, on a replay as on a write. `left_out` holds at most 10 items of at most 300 characters each, none blank and none repeated. The answer is `briefing`, `accept` or `send_back` (needs a note), with `file`, the left-out items to file by number from 1, none repeated; `file` goes only with a verdict, never with `drop` or on another kind. An amend's `left_out` replaces the list and moves `amend_seq`, so an answer that numbered the old list is `stale`; its `landed` does not, and is the one amend field read at its zero value, so `false` sets it. A briefing does not park | `open.json`, `answer.json`, `amend.json`, `transitions.json` | AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct); open points 2 and 3 as proposed |
| How a case changed since a form was drawn from it, `unchanged`, `moved` or `changed`, by the six steps of the README's Freshness section. A question-changing amend since is `changed`, except the human's amend of a message; a state passed twice, as in an answer and a reopen or a park and a resume, is `changed`; a skipped row since is `changed`, and one before the form was drawn is not counted | `freshness.json` | IN-05, IN-06, IN-13; design note `freshness-rule-design-2026-11-01`, decisions of 1 Nov |
| The stats of some cases over the last few UTC days: cases opened, answers given each day (a drop is not one; a case reopened and answered again counts twice) and the median time to answer, timed from the open or the last reopen and cut to whole seconds. Messages are left out | `stats.json` | TU stats strip; design note `tui-stats-design-2026-11-02`, decisions of 2 Nov (UTC days for now) |

## Counts

| File | First version | Now |
| --- | --- | --- |
| `transitions.json` | 105 | 197 |
| `open.json` | 63 | 141 |
| `answer.json` | 61 | 100 |
| `amend.json` | 59 | 112 |
| `fold.json` | 44 | 80 |
| `writer.json` | 50 | 51 |
| `freshness.json` | new | 38 |
| `stats.json` | new | 36 |
| `go-quirks.json` | 39 | removed |
| `store/version.json` | new | 6 |
| `store/changes.json` | new | 9 |
| `store/actors.json` | new | 4 |
| `store/listening.json` | new | 21 |

## `transitions.json`

### Changed

- `open: withdraw by the agent is accepted and the case is withdrawn` (was `open: withdraw is accepted and the case is withdrawn`): ST-14: withdraw has two authors now, so the description names the author.
- `answered: withdraw by the agent is refused` (was `answered: withdraw is refused`): ST-14: withdraw has two authors now, so the description names the author.
- `pickedup: withdraw by the agent is refused` (was `pickedup: withdraw is refused`): ST-14: withdraw has two authors now, so the description names the author.
- `closed: withdraw by the agent is refused` (was `closed: withdraw is refused`): ST-14: withdraw has two authors now, so the description names the author.
- `withdrawn: withdraw by the agent is refused` (was `withdrawn: withdraw is refused`): ST-14: withdraw has two authors now, so the description names the author.
- `parked: withdraw by the agent is refused` (was `parked: withdraw is refused`): ST-14: withdraw has two authors now, so the description names the author.
- `resume is written by the human and no one else` (was `resume is written by the agent or the human and no one else`): HU-05: resume is human-only.
- `withdraw is written by the agent or the human and no one else` (was `withdraw is written by the agent and no one else`): ST-14: the human writes withdraw too; AG-11: the agent's only from `open`; case Q8PA4: the human's from `open`, `answered` or `pickedup`.
- `a resumed case can be answered`: the resume at seq 3 is the human's, not the agent's. HU-05: resume is human-only.
- `open: amend by the agent is accepted and the case is open` (was `open: amend is accepted and the case is open`): Open and amend have two authors now, so the description names the author, as the withdraw vectors do since ST-14.
- `open: amend by the human is refused for its author` (was `amend is written by the agent, not the human`): Moved among the open state's vectors, where the matrix of a stuck case tries it; a stuck case's amend is the agent's (decided for Ryan by cases-sh, 27 Sep).
- `open: a second open by the agent is skipped` (was `open: a second open is skipped`): Open and amend have two authors now, so the description names the author, as the withdraw vectors do since ST-14.
- `answered: amend by the agent is refused` (was `answered: amend is refused`): Open and amend have two authors now, so the description names the author, as the withdraw vectors do since ST-14.
- `answered: a second open by the agent is skipped` (was `answered: a second open is skipped`): Open and amend have two authors now, so the description names the author, as the withdraw vectors do since ST-14.
- `pickedup: amend by the agent is refused` (was `pickedup: amend is refused`): Open and amend have two authors now, so the description names the author, as the withdraw vectors do since ST-14.
- `pickedup: a second open by the agent is skipped` (was `pickedup: a second open is skipped`): Open and amend have two authors now, so the description names the author, as the withdraw vectors do since ST-14.
- `closed: amend by the agent is refused` (was `closed: amend is refused`): Open and amend have two authors now, so the description names the author, as the withdraw vectors do since ST-14.
- `closed: a second open by the agent is skipped` (was `closed: a second open is skipped`): Open and amend have two authors now, so the description names the author, as the withdraw vectors do since ST-14.
- `withdrawn: amend by the agent is refused` (was `withdrawn: amend is refused`): Open and amend have two authors now, so the description names the author, as the withdraw vectors do since ST-14.
- `withdrawn: a second open by the agent is skipped` (was `withdrawn: a second open is skipped`): Open and amend have two authors now, so the description names the author, as the withdraw vectors do since ST-14.
- `parked: amend by the agent is refused` (was `parked: amend is refused`): Open and amend have two authors now, so the description names the author, as the withdraw vectors do since ST-14.
- `parked: a second open by the agent is skipped` (was `parked: a second open is skipped`): Open and amend have two authors now, so the description names the author, as the withdraw vectors do since ST-14.
- `an fyi is the agent's to open: one opened by the human is refused for its author` (was `open is written by the agent, not the human`): Open and amend belong to the side that opens the kind: the human's on a message, the agent's on every other kind (decided for Ryan by cases-sh, 27 Sep).
- `open is written by the agent or the human and no one else` (was `open is written by the agent and no one else`): Open has two authors now; any other is still refused.
- `amend is written by the agent or the human and no one else` (was `amend is written by the agent and no one else`): Amend has two authors now; any other is still refused.
- `answered: note without reopen is refused` (was `answered: note is accepted and the case is open`): accepted before; now a transition. Cases 6M6gx and AV8s1 (1 Oct): reopening a case is explicit: a note on an answered or picked-up case says `reopen`, and the writer refuses one that does not, as a transition.
- `pickedup: note without reopen is refused` (was `pickedup: note is accepted and the case is open`): accepted before; now a transition. Cases 6M6gx and AV8s1 (1 Oct): reopening a case is explicit: a note on an answered or picked-up case says `reopen`, and the writer refuses one that does not, as a transition.
- `a stored note on an answered case reopens it and clears the answer, with reopen or without` (was `a note on an answered case reopens it and clears the answer`): the note says `reopen`. Cases 6M6gx and AV8s1 (1 Oct): reopening a case is explicit: a note on an answered or picked-up case says `reopen`, and the writer refuses one that does not, as a transition; `fold.json` keeps a stored note without it.
- `a note on a picked-up case reopens it and clears the answer and the pickup`: the note says `reopen`, as a written one must now. Cases 6M6gx and AV8s1 (1 Oct): reopening a case is explicit: a note on an answered or picked-up case says `reopen`, and the writer refuses one that does not, as a transition.
- `a reopened case takes a new answer and closes on it`: the note says `reopen`, as a written one must now. Cases 6M6gx and AV8s1 (1 Oct): reopening a case is explicit: a note on an answered or picked-up case says `reopen`, and the writer refuses one that does not, as a transition.
- `a case a note reopened can be amended`: the note says `reopen`, as a written one must now. Cases 6M6gx and AV8s1 (1 Oct): reopening a case is explicit: a note on an answered or picked-up case says `reopen`, and the writer refuses one that does not, as a transition.

### Added

- `open: withdraw by the human is accepted and the case is withdrawn`: ST-14: the human writes withdraw too; case Q8PA4: the human's from `open`, `answered` or `pickedup`.
- `answered: withdraw by the human is accepted and the case is withdrawn` (was `answered: withdraw by the human is refused`): refused before; now accepted, and the case keeps its answer. ST-14: the human writes withdraw too; case Q8PA4: the human may also end a case an agent has, answered or picked up, such as one whose session died, while the agent's withdraw stays open-only (AG-11).
- `pickedup: withdraw by the human is accepted and the case is withdrawn` (was `pickedup: withdraw by the human is refused`): refused before; now accepted, and the case keeps its answer and its pickup. ST-14: the human writes withdraw too; case Q8PA4: the human may also end a case an agent has, answered or picked up, such as one whose session died, while the agent's withdraw stays open-only (AG-11).
- `closed: withdraw by the human is refused`: ST-14: the human writes withdraw too; case Q8PA4: the human's only from `open`, `answered` or `pickedup`.
- `withdrawn: withdraw by the human is refused`: ST-14: the human writes withdraw too; case Q8PA4: the human's only from `open`, `answered` or `pickedup`.
- `parked: withdraw by the human is refused`: ST-14: the human writes withdraw too; case Q8PA4: the human's only from `open`, `answered` or `pickedup`.
- `resume is written by the human, not the agent`: an agent resume of a parked case, which the state would allow. HU-05: resume is human-only.
- `open: a second open by the human is skipped`: A case takes one open, whichever author writes the second.
- `answered: amend by the human is refused for its author`: A stuck case's amend is the agent's (decided for Ryan by cases-sh, 27 Sep).
- `answered: a second open by the human is skipped`: A case takes one open, whichever author writes the second.
- `pickedup: amend by the human is refused for its author`: A stuck case's amend is the agent's (decided for Ryan by cases-sh, 27 Sep).
- `pickedup: a second open by the human is skipped`: A case takes one open, whichever author writes the second.
- `closed: amend by the human is refused for its author`: A stuck case's amend is the agent's (decided for Ryan by cases-sh, 27 Sep).
- `closed: a second open by the human is skipped`: A case takes one open, whichever author writes the second.
- `withdrawn: amend by the human is refused for its author`: A stuck case's amend is the agent's (decided for Ryan by cases-sh, 27 Sep).
- `withdrawn: a second open by the human is skipped`: A case takes one open, whichever author writes the second.
- `parked: amend by the human is refused for its author`: A stuck case's amend is the agent's (decided for Ryan by cases-sh, 27 Sep).
- `parked: a second open by the human is skipped`: A case takes one open, whichever author writes the second.
- `a sent message: amend by the human is accepted and the message is sent`: Case 8g2R0: the human can edit a message until it is picked up, with an amend.
- `a sent message: amend by the agent is refused for its author`: A message's open and amend are the human's (decided for Ryan by cases-sh, 27 Sep).
- `a sent message: answer is refused`: Case 8g2R0: a message takes no answer, and is never open or parked.
- `a sent message: pickup is accepted and the message is pickedup`: Case 8g2R0: the agent picks a sent message up; case rvRCl: the first to do so has it.
- `a sent message: note is refused`: Case 8g2R0: a message takes no note; its state refuses it first.
- `a sent message: close is refused`: Case 8g2R0: a message is closed once it is picked up.
- `a sent message: withdraw by the agent is refused`: AG-11: the agent withdraws only an open case, and a message is never open.
- `a sent message: withdraw by the human is accepted and the message is withdrawn`: Case 8g2R0: the human can withdraw a message until it is picked up.
- `a sent message: park is refused`: Case 8g2R0: a message takes no answer, and is never open or parked.
- `a sent message: resume by the human is refused`: Case 8g2R0: a message takes no answer, and is never open or parked.
- `a sent message: a second open by the human is skipped`: A case takes one open, whichever author writes the second.
- `a sent message: a second open by the agent is skipped`: A case takes one open, whichever author writes the second.
- `a picked-up message: amend by the human is refused`: Case 8g2R0: the human's edit is only while the message is sent.
- `a picked-up message: amend by the agent is refused for its author`: A message's open and amend are the human's (decided for Ryan by cases-sh, 27 Sep).
- `a picked-up message: answer is refused`: Case 8g2R0: a message takes no answer, and is never open or parked.
- `a picked-up message: pickup is refused`: Case 8g2R0: a message is picked up once, while it is sent.
- `a picked-up message: note is refused, as a message takes no note`: Case 8g2R0: there are no threads inside a message; a note asks for a new answer, which a message never has.
- `a picked-up message: close is accepted and the message is closed`: Case 8g2R0: the outcome comes back at close.
- `a picked-up message: withdraw by the agent is refused`: AG-11: the agent withdraws only an open case, and a message is never open.
- `a picked-up message: withdraw by the human is accepted and the message is withdrawn`: Case Q8PA4: the human's withdraw also ends a case an agent has, and it keeps its pickup.
- `a picked-up message: park is refused`: Case 8g2R0: a message takes no answer, and is never open or parked.
- `a picked-up message: resume by the human is refused`: Case 8g2R0: a message takes no answer, and is never open or parked.
- `a picked-up message: a second open by the human is skipped`: A case takes one open, whichever author writes the second.
- `a picked-up message: a second open by the agent is skipped`: A case takes one open, whichever author writes the second.
- `a closed message: amend by the human is refused`: Case 8g2R0: the human's edit is only while the message is sent.
- `a closed message: amend by the agent is refused for its author`: A message's open and amend are the human's (decided for Ryan by cases-sh, 27 Sep).
- `a closed message: answer is refused`: Case 8g2R0: a message takes no answer, and is never open or parked.
- `a closed message: pickup is refused`: Case 8g2R0: a message is picked up once, while it is sent.
- `a closed message: note is refused`: Case 8g2R0: a message takes no note; its state refuses it first.
- `a closed message: close is refused`: Case 8g2R0: a message is closed once it is picked up.
- `a closed message: withdraw by the agent is refused`: AG-11: the agent withdraws only an open case, and a message is never open.
- `a closed message: withdraw by the human is refused`: ST-14: the human's withdraw does not end a closed or withdrawn case.
- `a closed message: park is refused`: Case 8g2R0: a message takes no answer, and is never open or parked.
- `a closed message: resume by the human is refused`: Case 8g2R0: a message takes no answer, and is never open or parked.
- `a closed message: a second open by the human is skipped`: A case takes one open, whichever author writes the second.
- `a closed message: a second open by the agent is skipped`: A case takes one open, whichever author writes the second.
- `a withdrawn message: amend by the human is refused`: Case 8g2R0: the human's edit is only while the message is sent.
- `a withdrawn message: amend by the agent is refused for its author`: A message's open and amend are the human's (decided for Ryan by cases-sh, 27 Sep).
- `a withdrawn message: answer is refused`: Case 8g2R0: a message takes no answer, and is never open or parked.
- `a withdrawn message: pickup is refused`: Case 8g2R0: a message is picked up once, while it is sent.
- `a withdrawn message: note is refused`: Case 8g2R0: a message takes no note; its state refuses it first.
- `a withdrawn message: close is refused`: Case 8g2R0: a message is closed once it is picked up.
- `a withdrawn message: withdraw by the agent is refused`: AG-11: the agent withdraws only an open case, and a message is never open.
- `a withdrawn message: withdraw by the human is refused`: ST-14: the human's withdraw does not end a closed or withdrawn case.
- `a withdrawn message: park is refused`: Case 8g2R0: a message takes no answer, and is never open or parked.
- `a withdrawn message: resume by the human is refused`: Case 8g2R0: a message takes no answer, and is never open or parked.
- `a withdrawn message: a second open by the human is skipped`: A case takes one open, whichever author writes the second.
- `a withdrawn message: a second open by the agent is skipped`: A case takes one open, whichever author writes the second.
- `a message is the human's to open: one opened by the agent is refused for its author`: Case oPWlr: only the human sends a message (decided for Ryan by cases-sh, 27 Sep).
- `a decision is the agent's to open: one opened by the human is refused for its author`: Who opens a case is checked once its kind is known, so each kind the agent opens needs its own vector; before messages, one vector covered every kind (decided for Ryan by cases-sh, 27 Sep).
- `an approval is the agent's to open: one opened by the human is refused for its author`: As for a decision: each kind the agent opens has its own vector.
- `a signoff is the agent's to open: one opened by the human is refused for its author`: As for a decision: each kind the agent opens has its own vector.
- `a stuck case is the agent's to open: one opened by the human is refused for its author`: As for a decision: each kind the agent opens has its own vector.
- `a question is the agent's to open: one opened by the human is refused for its author`: As for a decision: each kind the agent opens has its own vector.
- `a decision is the agent's to amend: one amended by the human is refused for its author`: Who amends a case is checked once the case's kind is known, so each kind the agent opens needs its own vector; the matrix covers a stuck case's (decided for Ryan by cases-sh, 27 Sep).
- `an approval is the agent's to amend: one amended by the human is refused for its author`: As for a decision: each kind the agent opens has its own vector.
- `a signoff is the agent's to amend: one amended by the human is refused for its author`: As for a decision: each kind the agent opens has its own vector.
- `a question is the agent's to amend: one amended by the human is refused for its author`: As for a decision: each kind the agent opens has its own vector.
- `an fyi is the agent's to amend: one amended by the human is refused for its author`: As for a decision: each kind the agent opens has its own vector.
- `a message goes from sent to closed`: Case 8g2R0: a message is sent, picked up by an agent, and closed with the outcome; each event records the state before it.
- `answered: note with reopen is accepted and the case is open`: Cases 6M6gx and AV8s1 (1 Oct): reopening a case is explicit: a note on an answered or picked-up case says `reopen`, and the writer refuses one that does not, as a transition.
- `pickedup: note with reopen is accepted and the case is open`: Cases 6M6gx and AV8s1 (1 Oct): reopening a case is explicit: a note on an answered or picked-up case says `reopen`, and the writer refuses one that does not, as a transition.
- `open: note with reopen is refused, as there is nothing to reopen`: `reopen` on a case that is open or sent is `invalid`: there is nothing to reopen (cases 6M6gx and AV8s1).
- `a note without reopen is refused before its body is checked`: the write rule of `reopen` is checked with the state, so a blank note without it is a transition. Cases 6M6gx and AV8s1 (1 Oct): reopening a case is explicit: a note on an answered or picked-up case says `reopen`, and the writer refuses one that does not, as a transition.
- `a note with reopen and a blank body is refused for its body`: with `reopen`, the state allows the note, and its body is checked. Cases 6M6gx and AV8s1 (1 Oct): reopening a case is explicit: a note on an answered or picked-up case says `reopen`, and the writer refuses one that does not, as a transition.
- `open: amend by the agent with reopen is refused, as there is nothing to reopen`: `reopen` on a case that is open or sent is `invalid`: there is nothing to reopen (cases 6M6gx and AV8s1).
- `answered: amend by the agent with reopen is accepted and the case is open`: Cases 6M6gx and AV8s1 (1 Oct): the agent's amend with `reopen` reopens an answered or picked-up case, as a reopening note does.
- `pickedup: amend by the agent with reopen is accepted and the case is open`: Cases 6M6gx and AV8s1 (1 Oct): the agent's amend with `reopen` reopens an answered or picked-up case, as a reopening note does.
- `closed: amend by the agent with reopen is refused`: `reopen` takes only an answered or picked-up case. Cases 6M6gx and AV8s1 (1 Oct): the agent's amend with `reopen` reopens an answered or picked-up case, as a reopening note does.
- `withdrawn: amend by the agent with reopen is refused`: `reopen` takes only an answered or picked-up case. Cases 6M6gx and AV8s1 (1 Oct): the agent's amend with `reopen` reopens an answered or picked-up case, as a reopening note does.
- `parked: amend by the agent with reopen is refused`: `reopen` takes only an answered or picked-up case. Cases 6M6gx and AV8s1 (1 Oct): the agent's amend with `reopen` reopens an answered or picked-up case, as a reopening note does.
- `a sent message: amend by the human with reopen is refused, as there is nothing to reopen`: `reopen` on a case that is open or sent is `invalid`: there is nothing to reopen (cases 6M6gx and AV8s1).
- `a picked-up message: amend by the human with reopen is refused, as a message is not reopened`: a message waits on an agent, not on the human, so nothing asks the human again. Cases 6M6gx and AV8s1 (1 Oct): the agent's amend with `reopen` reopens an answered or picked-up case, as a reopening note does.
- `a briefing is the agent's to open: one opened by the human is refused for its author`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `a briefing is the agent's to amend: one amended by the human is refused for its author`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `a briefing case cannot be parked; only stuck cases park`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `a briefing case goes from open to closed`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `a briefing sent back is amended with reopen, judged again and closed`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `a picked-up briefing closes expired, with an outcome that says why`: AG-19: a briefing left picked up past the config file's `briefing-close-after` is closed as expired, not by the agent's judgement, and so wakes no wait (decided by Ryan, 1 Nov).
- `an expired close needs an outcome, as any close does`: AG-19: a briefing left picked up past the config file's `briefing-close-after` is closed as expired, not by the agent's judgement, and so wakes no wait (decided by Ryan, 1 Nov).
- `only a briefing closes expired: a stuck case's expired close is refused`: AG-19: a briefing left picked up past the config file's `briefing-close-after` is closed as expired, not by the agent's judgement, and so wakes no wait (decided by Ryan, 1 Nov).

### Removed

- `open: resume by the agent is refused`: 'resume is written by the human, not the agent' covers the agent's resume. HU-05: resume is human-only.
- `answered: resume by the agent is refused`: 'resume is written by the human, not the agent' covers the agent's resume. HU-05: resume is human-only.
- `pickedup: resume by the agent is refused`: 'resume is written by the human, not the agent' covers the agent's resume. HU-05: resume is human-only.
- `closed: resume by the agent is refused`: 'resume is written by the human, not the agent' covers the agent's resume. HU-05: resume is human-only.
- `withdrawn: resume by the agent is refused`: 'resume is written by the human, not the agent' covers the agent's resume. HU-05: resume is human-only.
- `parked: resume by the agent is accepted and the case is open`: 'resume is written by the human, not the agent' covers the agent's resume. HU-05: resume is human-only.
- `withdraw is written by the agent, not the human`: the human's withdraw is accepted now, from an `open`, `answered` or `pickedup` case; the '...: withdraw by the human ...' vectors cover it. ST-14: the human writes withdraw too; case Q8PA4.

## `open.json`

### Changed

- `the urgency is checked before every other rule: an unknown urgency on an open that breaks them all is unknown`: the open breaks the write rules as well: an option twice, a link of only white space, a link twice, and a worker, brief and context of only white space. AG-02: refused when written; a stored open still folds.
- `an open may have a body of only white space` (was `an open may have a body and context of only white space`): the context goes to vectors of its own. AG-02: a context of only white space is refused when written; a body is not on its list.
- `the open's actor is the case's actor and its event's`: the actor is an id in place of a name and a kind. The actor decision (case 2026-09-24T21-30-51Z-actor-clean-slate-events-hold-only-id-names-in): an event's actor is only an id.
- `an actor without an id is refused` (was `an actor with a blank name is refused`): an actor has no name now; one without an id names no one. The actor decision (case 2026-09-24T21-30-51Z-actor-clean-slate-events-hold-only-id-names-in): an event's actor is only an id.
- `a decision case opens with its options`: expects `recommend` 0, the case's new property, for an open that gives none. Case AXs1i: recommend is a field of the v1 contract.
- `the urgency is checked before every other rule: an unknown urgency on an open that breaks them all is unknown`: the open gives `recommend` 3 of its two options as well, so it breaks the recommend's rule too. Case AXs1i: the recommend is one of the open's rules, after its kind and urgency.

### Added

- `the kind is checked before the urgency: an open without a kind and with an unknown urgency is invalid`: AG-01: kind and urgency are required, as the title is; ST-05 keeps unknown for a value this version does not know. The kind is checked, then the urgency, then the open's other rules.
- `a stored open with a link of only white space folds`: AG-02: refused when written; a stored open still folds.
- `a stored open that gives a link twice folds`: AG-02: refused when written; a stored open still folds.
- `a worker of only white space is refused`: AG-02: refused when written; a stored open still folds.
- `a stored open with a worker of only white space folds`: AG-02: refused when written; a stored open still folds.
- `a brief of only white space is refused`: AG-02: refused when written; a stored open still folds.
- `a stored open with a brief of only white space folds`: AG-02: refused when written; a stored open still folds.
- `a context of only white space is refused`: AG-02: refused when written; a stored open still folds.
- `a stored open with a context of only white space folds`: AG-02: refused when written; a stored open still folds.
- `an empty worker is no worker`: AG-02 refuses a worker of only white space; an empty one reads as left out, as any field at its zero value does (case 2026-09-24T20-38-25Z-does-a-zero-value-field-count-as-given).
- `an actor id of other than 22 characters is refused`: a worker's name in place of an id. The actor decision (case 2026-09-24T21-30-51Z-actor-clean-slate-events-hold-only-id-names-in): an event's actor is only an id.
- `an actor id with a character outside base64url is refused`: + and / are base64, not base64url. The actor decision (case 2026-09-24T21-30-51Z-actor-clean-slate-events-hold-only-id-names-in): an event's actor is only an id.
- `an actor id is checked for its length and alphabet, and not decoded`: the last character is one that 128 bits never give. The actor decision (case 2026-09-24T21-30-51Z-actor-clean-slate-events-hold-only-id-names-in): an event's actor is only an id, checked as 22 base64url characters: the core never decodes it.
- `an actor id may hold - and _, the base64url characters that are not letters or digits`: an id ending in `-_`. The actor decision (case 2026-09-24T21-30-51Z-actor-clean-slate-events-hold-only-id-names-in): an actor id is 22 base64url characters. A gap cargo-mutants found in the vectors: the core had a unit test for it, and no vector (PR #3, the mutants triage).
- `a row field this version does not know is left out of the row, and the open folds`: AG-01: the CLI refuses a `--row` with an unknown field, and the core does not check for one. A gap the v1 stories sweep found: no vector pinned it. It is a replay, since an appended record names only the fields its event defines.
- `a decision case may recommend one of its options`: Case AXs1i: a decision's open may carry `recommend`, the option the agent recommends, numbered from 1.
- `a recommend past the last option is refused: other is not an option`: `recommend` 3 on two options, the number a choice of other would have. Case AXs1i: a recommend names one of the case's options.
- `a negative recommend is refused as invalid, as a negative choice is`: `recommend` is a whole number, as `choice` is, not a count, so -1 reads and names no option. Case AXs1i.
- `a recommend of 0 is one left out, even on a kind that takes none`: an fyi with `recommend` 0 opens. A field at its zero value reads as a field left out (case 2026-09-24T20-38-25Z-does-a-zero-value-field-count-as-given); case AXs1i.
- `a recommend on an approval case is refused`: Case AXs1i: only a decision takes a recommend; an approval's rows are not options.
- `a recommend on a signoff case is refused`: Case AXs1i: only a decision takes a recommend; accept and changes are not options.
- `a recommend on a stuck case is refused`: Case AXs1i: only a decision takes a recommend.
- `a recommend on a question case is refused`: Case AXs1i: only a decision takes a recommend.
- `a recommend on an fyi case is refused`: Case AXs1i: only a decision takes a recommend.
- `a stored open whose recommend is not one of its options does not fold: the rule is a replay's as well as a write's`: a replay. ST-02: the writer uses the fold's rules; the recommend's are not among the write rules of AG-02, so a stored open that breaks them is skipped. Case AXs1i.
- `a stored open with a recommend on an fyi case does not fold: only a decision takes one, on a replay as on a write`: a replay, with `recommend` 1 on an fyi. ST-02: only a decision takes a recommend, and that rule is not among the write rules of AG-02, so a stored open that breaks it is skipped. The vectors that refuse a recommend on another kind are all appends; this pins the replay. Case AXs1i.
- `a message opens sent, to the worker it names`: Case 8g2R0: a message starts sent, not open; Case rvRCl: a message goes to a worker or a label.
- `a message may go to a label and no worker`: Case rvRCl: a message goes to a worker or a label.
- `a message may go to a worker and to labels`: Case rvRCl: a message goes to a worker or a label, or both.
- `a message to no worker and no label is refused`: Case rvRCl: a message goes to a worker or a label; there is no any agent.
- `a stored message whose worker is only white space goes to no one, and does not fold`: Case rvRCl: a message goes to a worker or a label: a worker of only white space names no one, on a replay too, where the write rule of AG-02 does not run.
- `the kind is checked before the author: the human's open of an unknown kind is unknown`: The kind decides who opens it, so it is checked first (decided for Ryan by cases-sh, 27 Sep).
- `the kind is checked before the author: the human's open without a kind is invalid`: The kind decides who opens it, so it is checked first (decided for Ryan by cases-sh, 27 Sep).
- `the author is checked before the urgency: the agent's open of a message with an unknown urgency is refused for its author`: Who opens a kind is checked straight after the kind (decided for Ryan by cases-sh, 27 Sep).
- `the author is checked before the open's other rules: the human's open of an fyi with a blank title is refused for its author`: Who opens a kind is checked straight after the kind, before the open's other rules (decided for Ryan by cases-sh, 27 Sep).
- `options on a message are refused`: AG-01: options go only on a decision.
- `a recommend on a message is refused`: Case AXs1i: only a decision takes a recommend.
- `rows on a message are refused`: AG-01: rows go only on an approval.
- `re names the case the open is about`: Case 8g2R0: the agent's questions go into ordinary cases linked to the message by re.
- `a re of only white space is refused`: Case 8g2R0: the agent's questions go into ordinary cases linked to the message by re; like for, it is not blank when given.
- `a stored open with a re of only white space does not fold: the rule is a replay's as well as a write's`: Case 8g2R0: the agent's questions go into ordinary cases linked to the message by re; the rule is not one of AG-02's write rules, so a replay applies it too (ST-02).
- `a message may name the case it is about in re`: Case aC8tN: a message sent with message this agent is about the case it was sent from.
- `the actor is checked before the author of an open: the human's open of an fyi whose actor has a bad id is invalid`: ST-02: the actor is checked before the open's own checks, who opens its kind among them (decided for Ryan by cases-sh, 27 Sep).
- `a row may say the human carries it out`: an approval with one row for the agent and one with `by` `human`; the case's rows keep it. Case `i2lDf`: approval rows say who carries them out.
- `a row whose by is not human is refused`: `by` `robot`. Case `i2lDf`: `human` is the only value `by` takes.
- `a row whose by is agent is refused: a row the agent carries out leaves by out`: Case `i2lDf`: the agent's row has no `by`, so there is one way to write it.
- `a row whose by is Human, in another case, is refused`: Case `i2lDf`: `by` is matched exactly, as a kind is.
- `a stored open with a row whose by is not human does not fold: the rule is a replay's as well as a write's`: a replay. ST-02: the rule is not one of AG-02's write rules, so a replay applies it too. Case `i2lDf`.
- `a briefing opens with its body, landed and its left-out items`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `a briefing's landed is false when left out, and it may leave nothing out`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `a briefing with no body is refused`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `a briefing whose body is only white space is refused`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `a stored briefing with a blank body does not fold`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `landed on a decision is refused`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `left_out on an fyi is refused`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `left_out on a signoff is refused`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `a stored signoff with landed does not fold`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `a stored question with left_out does not fold`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `landed false on another kind is a field left out`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `a briefing with options is refused`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `a briefing with a recommend is refused`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `a briefing with rows is refused`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `a briefing may leave out 10 items`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `a briefing that leaves out 11 items is refused`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `a stored briefing that leaves out 11 items does not fold`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `a left-out item of 300 characters is taken, counted in characters, not bytes`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `a left-out item of 301 characters is refused`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `a stored briefing with a left-out item of 301 characters does not fold`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `a left-out item of only white space is refused`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `an empty left-out item is refused`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `a left-out item given twice is refused`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `a stored briefing with a left-out item given twice does not fold`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `a left-out item that differs from another only in case is taken`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).

### Moved here

- `an open without a kind is refused` (was `an open without a kind is refused as unknown`), from `go-quirks.json`: unknown becomes invalid. AG-01: kind and urgency are required, as the title is; ST-05 keeps unknown for a value this version does not know.
- `an open without an urgency is refused` (was `an open without an urgency is refused as unknown`), from `go-quirks.json`: unknown becomes invalid. AG-01: kind and urgency are required, as the title is; ST-05 keeps unknown for a value this version does not know.
- `an option given twice is refused` (was `an open may give an option twice, which an amend may not`), from `go-quirks.json`: accepted before; now invalid. AG-02: refused when written; a stored open still folds.
- `a stored open that gives an option twice folds` (was `a stored open that gives an option twice, a link twice and a blank link folds`), from `fold.json`: split: the links are in vectors of their own. AG-02: refused when written; a stored open still folds.
- `a link of only white space is refused` (was `an open may have a link of only white space, which an amend may not`), from `go-quirks.json`: accepted before; now invalid. AG-02: refused when written; a stored open still folds.
- `a link given twice is refused` (was `an open may give a link twice, which an amend may not`), from `go-quirks.json`: accepted before; now invalid. AG-02: refused when written; a stored open still folds.
- `an appended open is checked for its actor before its own rules` (was `an appended open is checked for its own rules before its actor`), from `go-quirks.json`: unknown becomes invalid: the actor is checked first, as on a replay; the actor that is refused has a bad id, where it had a blank name. ST-02: the writer checks an open in the replay's order. The actor decision (case 2026-09-24T21-30-51Z-actor-clean-slate-events-hold-only-id-names-in): an event's actor is only an id.
- `a stored open is checked for its actor before its own rules`, from `go-quirks.json`: the actor that is refused has a bad id, where it had a blank name. ST-02: the writer checks an open in the replay's order. The actor decision (case 2026-09-24T21-30-51Z-actor-clean-slate-events-hold-only-id-names-in): an event's actor is only an id.

### Removed

- `an actor may be of any kind`: an actor has no kind now. The actor decision (case 2026-09-24T21-30-51Z-actor-clean-slate-events-hold-only-id-names-in): an event's actor is only an id.
- `an actor without a kind is refused`: an actor has no kind now. The actor decision (case 2026-09-24T21-30-51Z-actor-clean-slate-events-hold-only-id-names-in): an event's actor is only an id.
- `an actor with a kind of only white space is refused`: an actor has no kind now. The actor decision (case 2026-09-24T21-30-51Z-actor-clean-slate-events-hold-only-id-names-in): an event's actor is only an id.

## `answer.json`

### Changed

- `amended: a verdict on every row, the added one too`: the answer carries amend_seq 2, the amend its writer saw. ST-04: an answer carries the amend seq its writer saw, and one below the case's is refused.
- `amended: verdicts on only the rows the case opened with are refused`: the answer carries amend_seq 2, the amend its writer saw. ST-04: an answer carries the amend seq its writer saw, and one below the case's is refused.
- `amended: a row the amended case does not have is refused`: the answer carries amend_seq 2, the amend its writer saw. ST-04: an answer carries the amend seq its writer saw, and one below the case's is refused.
- `amended: a choice of the added option`: the answer carries amend_seq 2, the amend its writer saw. ST-04: an answer carries the amend seq its writer saw, and one below the case's is refused.
- `amended: a choice past the added option is refused`: the answer carries amend_seq 2, the amend its writer saw. ST-04: an answer carries the amend seq its writer saw, and one below the case's is refused.

### Added

- `amended: an answer that saw an earlier amend but not the latest is refused`: amend_seq 2 on a case whose amend_seq is 3, from a body change. ST-04: an answer carries the amend seq its writer saw, and one below the case's is refused.
- `amended: an answer that names a later amend than the case's is refused`: amend_seq 3 on a case whose amend_seq is 2. ST-04: an answer carries the amend seq its writer saw, and one below the case's is refused; one above the case's, which no writer can have seen, is invalid (case 2026-09-24T20-38-01Z-st-04-error-for-an-answer-that-missed-the).
- `amended: an answer need not see an amend of links, labels or context, or of the body the case has`: an answer with no amend_seq after such an amend is accepted. ST-03, ST-04: only a new body text, added options or added rows move amend_seq.
- `amended: a stored answer that did not see the amend is skipped`: a replay: the answer's row is skipped as stale. ST-04: an answer carries the amend seq its writer saw, and one below the case's is refused; ST-02: the fold checks it as the writer does.
- `an amend seq below 0 is malformed, not stale: it is a count`: an appended answer with amend_seq -1. It was stale, as below the case's 0. ST-04; amend_seq is a count, so a negative one is of the wrong type (case 2026-09-24T22-54-01Z-negative-amend-seq-on-an-answer-which-error).
- `a stored answer with an amend seq below 0 is skipped as malformed`: the same answer as a stored row. ST-04, ST-02: the fold reads it as the writer does (case 2026-09-24T22-54-01Z-negative-amend-seq-on-an-answer-which-error).
- `approval: a case of one row takes a verdict on it`: an approval with a single row, answered. A gap cargo-mutants found in the vectors: the core had a unit test for it, and no vector (PR #3, the mutants triage).
- `an appended answer's amend seq is checked before the state: one that missed an amend on an answered case is stale`: ST-04: the writer's side of `fold.json` "an answer's amend seq is checked before the state". A gap the v1 stories sweep found: only the replay pinned it.
- `decision: a choice of an option the case does not recommend`: the case recommends 1, and the answer chooses 2. Case AXs1i: the recommend is shown, never enforced; HU-01: a decision's answer is any option.
- `amended: an answer need not see a new recommend`: an amend changes the recommend from 1 to 2, and an answer without an amend seq, choosing 1, is taken. ST-04, case AXs1i: the recommend is advice about the question, not the question, and moving `amend_seq` for it would split an older build, which skips an amend that sets only a recommend, from this one: each would skip the other's answers as stale (ST-07 does not catch it).
- `briefing: an accept`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `briefing: an accept that files left-out items`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `briefing: a send back with a reason`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `briefing: a send back that files an item`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `briefing: a send back without a reason is refused`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `briefing: a send back whose reason is only white space is refused`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `briefing: a verdict other than accept or send_back is refused`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `briefing: a verdict that differs from accept only in case is refused`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `briefing: a signoff's verdict is refused`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `briefing: file without a verdict is refused`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `briefing: a drop`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `briefing: file beside drop is refused`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `briefing: a verdict beside drop is refused`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `briefing: file 0 names no item and is refused`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `briefing: a file past the last item is refused`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `briefing: a file below 1 is refused`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `briefing: an item filed twice is refused`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `briefing: file on a briefing that left nothing out is refused`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `briefing: an empty file list files nothing`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `signoff: file is refused`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `signoff: a briefing's verdict is refused`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `a stored briefing answer that files an item twice is skipped`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `briefing: an answer that ticked items from the old left-out list is stale`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `briefing: an answer files by the left-out list as amended`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `briefing: an answer need not see an amend of landed`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).

### Moved here

- `amended: an answer that did not see the amend is refused` (was `an answer that did not see an amend is accepted when it sends no revision`), from `go-quirks.json`: accepted before; now refused as stale, with no at_revision. ST-04: an answer carries the amend seq its writer saw, and one below the case's is refused.
- `a response field at its zero value is not set: other false beside a choice`, from `go-quirks.json`: moved as it is. A field at its zero value reads as a field left out (case 2026-09-24T20-38-25Z-does-a-zero-value-field-count-as-given).
- `a response field at its zero value is not set: ack false beside drop`, from `go-quirks.json`: moved as it is. A field at its zero value reads as a field left out (case 2026-09-24T20-38-25Z-does-a-zero-value-field-count-as-given).
- `a response field at its zero value is not set: no rows beside text`, from `go-quirks.json`: moved as it is. A field at its zero value reads as a field left out (case 2026-09-24T20-38-25Z-does-a-zero-value-field-count-as-given).

## `amend.json`

### Changed

- `a context replaces the case's and leaves amend_seq alone` (was `a context replaces the case's`): amend_seq 2 becomes 0. ST-04: only a new body text, added options or added rows move amend_seq.
- `links are added after the case's and leave amend_seq alone` (was `links are added after the case's`): amend_seq 2 becomes 0. ST-04: only a new body text, added options or added rows move amend_seq.
- `the body the case has is accepted beside a link and leaves amend_seq alone` (was `the body the case has is accepted beside a link`): amend_seq 2 becomes 0. ST-04: only a new body text, added options or added rows move amend_seq.
- `a context that differs from the case's only in white space is a change`: amend_seq 2 becomes 0: it is a change, but not to the question. ST-04: only a new body text, added options or added rows move amend_seq.
- `a link that differs from the case's only in case is new`: amend_seq 2 becomes 0. ST-04: only a new body text, added options or added rows move amend_seq.
- `each amend changes only what it sets, and the open event keeps what it said`: amend_seq 3 becomes 2: the amend at seq 3 only adds a link. ST-04: only a new body text, added options or added rows move amend_seq.
- `amend_seq leaves out an amend that sets the body the case has beside labels` (was `amend_seq counts an amend that sets the body the case has beside labels`): amend_seq 2 becomes 0. ST-04: only a new body text, added options or added rows move amend_seq.
- `an amend with only an urgency, a field an amend does not have, is empty and refused` (was `an amend with only a title and an urgency, fields an amend does not have, is empty and refused`): the title is taken out, as an amend has one now. Case AV8s1 (1 Oct): an amend may replace the title, as it may the body.
- `an amend's urgency and for are ignored, and the body and title beside them are applied` (was `an amend's title, urgency and for are ignored, and the body beside them is applied`): the title is New title, not the open's. Case AV8s1 (1 Oct): an amend may replace the title, as it may the body. No stored amend has a title: the writer stores a record as it reads it, and dropped the field before.

### Added

- `a recommend replaces the case's and leaves amend_seq alone`: the recommend goes from 1 to 2, and amend_seq stays 0. Case AXs1i: an amend may change the recommend; ST-04: it does not change the question, so an older build, which skips the amend as changing nothing, works out the same amend_seq and takes the same answers.
- `a recommend may name an option the same amend adds`: an amend adds a third option and recommends it. Case AXs1i: the recommend is checked against the options as amended.
- `a recommend past the options as amended is refused`: an amend adds a third option and recommends a fourth. Case AXs1i: the recommend is checked against the options as amended.
- `the recommend the case has, alone, changes nothing and is refused`: as the body the case has is. AG-07: an amend that changes nothing is refused, so the same amend sent twice writes one event; case AXs1i.
- `the recommend the case has is accepted beside a link and leaves amend_seq alone`: as the body the case has is, and a new recommend would leave it alone too. ST-04: only a change to the question moves amend_seq; case AXs1i.
- `a recommend of 0 is one left out, so an amend cannot take the case's away`: an amend of a label and `recommend` 0 leaves the case's recommend 2. AG-07: nothing is removed; a field at its zero value reads as a field left out (case 2026-09-24T20-38-25Z-does-a-zero-value-field-count-as-given); case AXs1i.
- `a recommend on a case that is not a decision is refused`: a signoff. Case AXs1i: only a decision takes a recommend, on an amend as on an open.
- `the human's amend of a sent message replaces its body, and the message stays sent`: Case 8g2R0: the human can edit a message until it is picked up, with an amend.
- `the human's amend of a sent message adds labels`: Case 8g2R0: an edit may add labels; ST-04: labels leave amend_seq alone.
- `options on a message are refused`: AG-07: options go only on a decision, on an amend as on an open.
- `the author is checked before the amend's rules: the agent's amend of a sent message with nothing in it is refused for its author`: An amend's author is checked once the case is known, before its state and its rules (decided for Ryan by cases-sh, 27 Sep).
- `the actor is checked before the author of an amend: the human's amend of a stuck case whose actor has a bad id is invalid`: ST-02: the actor is checked before an amend's author (decided for Ryan by cases-sh, 27 Sep).
- `an amend may add a row the human carries out`: the added row keeps `by` `human`, and moves `amend_seq` as any added row does. Case `i2lDf`: approval rows say who carries them out; ST-04.
- `an amend that adds a row whose by is not human is refused`: `by` `robot`. Case `i2lDf`: an amend's rows are checked as an open's are.
- `a title replaces the case's and leaves amend_seq alone`: Case AV8s1 (1 Oct): an amend may replace the title, as it may the body. ST-04: like the recommend, a title is not the question, and a build from before the field skips the amend as changing nothing.
- `a title of only white space is refused`: Case AV8s1 (1 Oct): an amend may replace the title, as it may the body; blank as an open's title is.
- `the title the case has, alone, changes nothing and is refused`: as the body the case has is. AG-07; Case AV8s1 (1 Oct): an amend may replace the title, as it may the body.
- `a title that differs from the case's only in white space is a change`: as a body is. Case AV8s1 (1 Oct): an amend may replace the title, as it may the body.
- `the human's amend of a sent message may replace its title`: Case 8g2R0: the human edits a message with an amend; Case AV8s1 (1 Oct): an amend may replace the title, as it may the body.
- `a dropped link is taken off the case and leaves amend_seq alone`: Case AV8s1 (1 Oct): an amend may take links off the case with `drop_links`. ST-04: links are not the question.
- `links are dropped before the amend's links are added after the case's`: the links left keep their order. Case AV8s1 (1 Oct): an amend may take links off the case with `drop_links`.
- `every link may be dropped`: Case AV8s1 (1 Oct): an amend may take links off the case with `drop_links`.
- `a dropped link the case does not have is refused`: Case AV8s1 (1 Oct): an amend may take links off the case with `drop_links`.
- `a dropped link that differs from the case's only in case is refused`: links are matched exactly. Case AV8s1 (1 Oct): an amend may take links off the case with `drop_links`.
- `a link dropped twice is refused`: as a link added twice is. Case AV8s1 (1 Oct): an amend may take links off the case with `drop_links`.
- `a link both added and dropped is refused`: Case AV8s1 (1 Oct): an amend may take links off the case with `drop_links`.
- `a blank dropped link is refused`: the case has no such link. Case AV8s1 (1 Oct): an amend may take links off the case with `drop_links`.
- `an amend with reopen points a picked-up signoff at the new draft and asks again`: the signoff flow on `changes`: one amend with `reopen` replaces the title and body, drops the old link and adds the new one. Cases 6M6gx and AV8s1 (1 Oct): the agent's amend with `reopen` reopens an answered or picked-up case, as a reopening note does.
- `a case an amend reopened takes an answer to the amended question`: a replay. Cases 6M6gx and AV8s1 (1 Oct): the agent's amend with `reopen` reopens an answered or picked-up case, as a reopening note does.
- `an amend with reopen that leaves the question as it was keeps amend_seq`: Cases 6M6gx and AV8s1 (1 Oct): the agent's amend with `reopen` reopens an answered or picked-up case, as a reopening note does; ST-04: reopening is not a change to the question.
- `an amend with only reopen changes nothing and is refused`: Cases 6M6gx and AV8s1 (1 Oct): the agent's amend with `reopen` reopens an answered or picked-up case, as a reopening note does; an amend that changes nothing is refused (AG-07), so a reopen says what changed. A note with `reopen` asks again without a change.
- `a refused amend with reopen leaves the answer and the pickup`: Cases 6M6gx and AV8s1 (1 Oct): the agent's amend with `reopen` reopens an answered or picked-up case, as a reopening note does.
- `an amend of an answered case is refused without reopen, before its own rules`: the state is checked before the amend's rules. Cases 6M6gx and AV8s1 (1 Oct): the agent's amend with `reopen` reopens an answered or picked-up case, as a reopening note does.
- `an amend with reopen on an open case is refused before its own rules`: `reopen` on a case that is open or sent is `invalid`: there is nothing to reopen (cases 6M6gx and AV8s1).
- `a briefing's left-out list is replaced whole and moves amend_seq`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `a briefing's left-out list may be set where it had none`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `the left-out list the case has, alone, changes nothing and is refused`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `the left-out list the case has is accepted beside a link and leaves amend_seq alone`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `an empty left-out list is one left out, so an amend cannot clear the case's`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `landed replaces the case's and leaves amend_seq alone`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `landed false replaces the case's true: on an amend it is not a field left out`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `the landed the case has, alone, changes nothing and is refused`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `landed on an amend of a decision is refused`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `landed false on an amend of a question is refused`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `left_out on an amend of a question is refused`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `a stored amend with left_out on a question is skipped`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `an amend that leaves out 11 items is refused`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `an amend's left-out item given twice is refused`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `an amend's blank left-out item is refused`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `an amend's left-out item of 301 characters is refused`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `an amend of a briefing's body to only white space is refused`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `an amend of a briefing's options is refused`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `an amend of a briefing's recommend is refused`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).
- `a briefing sent back is reopened by an amend of its body and left-out list`: AG-19, design note `briefing-kind-design-2026-10-30` (discovery with Ryan, 30 Oct).

### Removed

- `amend_seq counts an amend that sets the context the case has beside labels`: no context counts now, whatever its text; 'a context replaces the case's and leaves amend_seq alone' covers it. ST-04: only a new body text, added options or added rows move amend_seq.

## `fold.json`

### Changed

- `rows fold in seq order and each event records the state before it`: the resume at seq 7 is the human's, not the agent's. HU-05: resume is human-only.
- `an actor's fields other than its id are left out, and kept in the event's data` (was `an actor from a later build folds and keeps its extra fields`): a name and a kind beside the id are read as fields this version does not know. The actor decision (case 2026-09-24T21-30-51Z-actor-clean-slate-events-hold-only-id-names-in): an event's actor is only an id.
- `each event keeps its actor, and the answer keeps the human's`: each actor is an id in place of a name and a kind. The actor decision (case 2026-09-24T21-30-51Z-actor-clean-slate-events-hold-only-id-names-in): an event's actor is only an id.
- `a row whose actor has no id is skipped` (was `a row whose actor has no name is skipped`): the actor has a name and a kind, as before, and no id. The actor decision (case 2026-09-24T21-30-51Z-actor-clean-slate-events-hold-only-id-names-in): an event's actor is only an id.
- `the actor is checked before the state`: the actor that is refused has a bad id, where it had a blank name. The actor decision (case 2026-09-24T21-30-51Z-actor-clean-slate-events-hold-only-id-names-in): an event's actor is only an id.
- `the actor is checked before the open: an answer before the open whose actor has a bad id is invalid` (was `the actor is checked before the open: an answer with a nameless actor before the open is invalid`): the actor that is refused has a bad id, where it had no name. The actor decision (case 2026-09-24T21-30-51Z-actor-clean-slate-events-hold-only-id-names-in): an event's actor is only an id.
- `an open of an fyi by the human does not fold: only a message is the human's to open` (was `an open by the human does not fold`): Open and amend belong to the side that opens the kind: the human's on a message, the agent's on every other kind (decided for Ryan by cases-sh, 27 Sep).

### Added

- `a key given twice in an object inside the record is malformed`: ST-06: a key given twice is malformed, at any depth.
- `an actor of null is malformed`: ST-06: null is not a field left out.
- `a time of null is malformed`: ST-06: null is not a field left out.
- `a space in place of the T, which RFC 3339's grammar does not allow, is malformed`: ST-06: times are read as RFC 3339 has them. Some time libraries take a space; RFC 3339's grammar does not.
- `a time keeps its fraction of a second, with only the digits it needs`: The README's rule that a time is written in UTC with only the fractional digits it needs had no vector once 'records an earlier build wrote, byte for byte, ...' went.
- `an answer's amend seq is checked before its rules: one that missed an amend and chooses no option is stale`: ST-04: an answer carries the amend seq its writer saw, and one below the case's is refused, as stale, before the state and the answer's rules (case 2026-09-24T20-38-01Z-st-04-error-for-an-answer-that-missed-the).
- `an answer's amend seq is checked before the state: one that missed an amend on an answered case is stale`: ST-04: an answer carries the amend seq its writer saw, and one below the case's is refused, as stale, before the state and the answer's rules (case 2026-09-24T20-38-01Z-st-04-error-for-an-answer-that-missed-the), as AG-12 has a stale revision.
- `-0 is a whole number, 0: an amend seq of -0 reads as 0`: serde_json reads the integer `-0` as the float -0.0; the core reads it as 0. ST-06; from the review of the core crate (PR #2), which fixed the reader with a unit test and suggested a vector.
- `-0.0 has a fraction part, so it is malformed where a whole number goes`: as `2.0` is. ST-06; from the review of the core crate (PR #2).
- `an offset of +23:59, the largest RFC 3339 allows, is read`: the edge beside `+24:00` and a minute of 60, which are malformed. ST-06. A gap cargo-mutants found in the vectors: the core had a unit test for it, and no vector (PR #3, the mutants triage).
- `a recommend that is a string is malformed, not a number`: `"2"` where a whole number goes. ST-06: a field of the wrong type is malformed; case AXs1i: `recommend` is a whole number.
- `a stored open's recommend is the case's, and a stored amend without one leaves it`: a replay. Case AXs1i: a stored record with `recommend` folds, and a record without one, as builds before it wrote, reads as none (ST-05).
- `an open without a recommend, as builds before the field wrote one, recommends none`: a replay: `recommend` 0. ST-05: a record without a field reads as the field left out; case AXs1i.
- `an open of a message by the agent does not fold: a message is the human's to open`: A message is the human's to open, on a replay as on a write (decided for Ryan by cases-sh, 27 Sep).
- `the record is read before who may open its kind: the human's open of an fyi that does not read is malformed`: Either author passes the first check of an open's author, so its record is read before its kind decides who opens it (decided for Ryan by cases-sh, 27 Sep).
- `the record is read before who may amend the case: the human's amend of a stuck case that does not read is malformed`: Either author passes the first check of an amend's author, so its record is read before the case's kind decides who amends it (decided for Ryan by cases-sh, 27 Sep).
- `a stored amend of a stuck case by the human is skipped for its author`: A stuck case's amend is the agent's, on a replay as on a write (decided for Ryan by cases-sh, 27 Sep).
- `a stored amend of a message by the agent is skipped for its author`: A message's amend is the human's, on a replay as on a write (decided for Ryan by cases-sh, 27 Sep).
- `a stored note without reopen on an answered case reopens it, as notes did before the field`: Cases 6M6gx and AV8s1 (1 Oct): reopening a case is explicit: a note on an answered or picked-up case says `reopen`, and the writer refuses one that does not, as a transition. The rule is the writer's alone: every note stored before the field reopened its case without it, and a replay keeps them as they were.
- `a stored note with reopen on an open case is skipped`: `reopen` on a case that is open or sent is `invalid`: there is nothing to reopen (cases 6M6gx and AV8s1), on a replay as on a write.
- `a stored expired close of a stuck case is skipped`: AG-19: a briefing left picked up past the config file's `briefing-close-after` is closed as expired, not by the agent's judgement, and so wakes no wait (decided by Ryan, 1 Nov).

### Moved here

- `field names are matched exactly: Kind is not kind, and ACK is not ack` (was `field names match whatever their case`), from `go-quirks.json`: the open has no kind and the answer no ack, so both are invalid; a second open lets the case fold. ST-06: field names match exactly.
- `fields whose names differ only in case are two fields, not one given twice` (was `fields whose names differ only in case are one field, and the last one wins`), from `go-quirks.json`: Choice is a field this version does not know, so the answer is choice 1. ST-06: field names match exactly.
- `a key given twice is malformed` (was `when a field is given twice the last one wins`), from `go-quirks.json`: choice 2 before; now the row is skipped as malformed. ST-06: a key given twice is malformed.
- `a record of null is malformed` (was `a record of null reads as an empty record`), from `go-quirks.json`: an empty pickup before; now the row is skipped as malformed. ST-06: null is not a field left out.
- `a field of null is malformed` (was `a field of null reads as a field not given`), from `go-quirks.json`: an answer with no note before; now malformed. ST-06: null is not a field left out.
- `a title of null is malformed, not a missing title` (was `a title of null is a missing title, which breaks the open's rules rather than being malformed`), from `go-quirks.json`: invalid becomes malformed. ST-06: null is not a field left out.
- `a response field of null is malformed` (was `a response field of null is not set`), from `go-quirks.json`: an ack before; now malformed. ST-06: null is not a field left out.
- `a lower-case z, which RFC 3339 allows, is read` (was `a lower-case z, which RFC 3339 allows, is malformed`), from `go-quirks.json`: malformed before; now read. ST-06: times are read as RFC 3339 has them.
- `a lower-case t, which RFC 3339 allows, is read` (was `a lower-case t, which RFC 3339 allows, is malformed`), from `go-quirks.json`: malformed before; now read. ST-06: times are read as RFC 3339 has them.
- `a time written with a JSON escape is read as the string it spells` (was `a time written with a JSON escape is malformed, though the string is the same`), from `go-quirks.json`: malformed before; now read. ST-06: times are read as RFC 3339 has them.
- `a leap second is malformed, though RFC 3339 allows one at the end of some days` (was `a leap second, which RFC 3339 allows, is malformed`), from `go-quirks.json`: the description only: a leap second stays malformed, as no clock the writer reads gives one. ST-06: times are read as RFC 3339 has them; a leap second is malformed (case 2026-09-24T21-24-26Z-is-a-leap-second-23-59-60-read).
- `an hour of one digit, which RFC 3339 does not allow, is malformed` (was `an hour of one digit, which RFC 3339 does not allow, is read`), from `go-quirks.json`: read before; now malformed. ST-06: times are read as RFC 3339 has them.
- `a comma before the fraction, which RFC 3339 does not allow, is malformed` (was `a comma before the fraction, which RFC 3339 does not allow, is read`), from `go-quirks.json`: read before; now malformed. ST-06: times are read as RFC 3339 has them.
- `an offset of +24:00, which RFC 3339 does not allow, is malformed` (was `an offset of +24:00, which RFC 3339 does not allow, is read`), from `go-quirks.json`: read before; now malformed. ST-06: times are read as RFC 3339 has them.
- `an offset with a minute of 60, which RFC 3339 does not allow, is malformed` (was `an offset with a minute of 60, which RFC 3339 does not allow, is read`), from `go-quirks.json`: read before; now malformed. ST-06: times are read as RFC 3339 has them.
- `a record without its time folds, with no time`, from `go-quirks.json`: moved as it is. ST-06: a missing time is left out, not written as a zero time.
- `an event without a time leaves updated_at as it was`, from `go-quirks.json`: moved as it is. ST-06: a missing time is left out.
- `a time in year 1 is a time like any other` (was `the zero time reads as no time`), from `go-quirks.json`: no time before; now the answer and its event keep 0001-01-01T00:00:00Z. ST-06: there is no zero time.

### Moved away

- `a stored open that gives an option twice, a link twice and a blank link folds`: to `open.json` as `a stored open that gives an option twice folds`.

### Removed

- `an open written before labels existed folds`: an open without labels is covered by every open vector that expects labels []. ST-05: no record from before this codebase is read.
- `records an earlier build wrote, byte for byte, fold as that build showed them`: 'a time keeps its fraction of a second, with only the digits it needs' keeps the part about fractions. ST-05: no record from before this codebase is read.

## `writer.json`

### Changed

- `stale is checked before the state`: the resume at seq 3 is the human's, so the case is open and a resume is refused by its state. HU-05: resume is human-only.
- `stale is checked before the actor: an answer with a bad actor id at an earlier revision is stale` (was `stale is checked before the actor: an answer with a nameless actor at an earlier revision is stale`): the actor has a bad id, where it had no name. The actor decision (case 2026-09-24T21-30-51Z-actor-clean-slate-events-hold-only-id-names-in): an event's actor is only an id.
- `stale is checked before the record's rules`: the answer carries amend_seq 2, the amend its writer saw, so only its at_revision makes it stale. ST-04: without it, the answer would be stale for its amend seq alone, and the vector would not show at_revision checked first.
- `an append is numbered after the highest seq`: the note on the picked-up case says `reopen`. Cases 6M6gx and AV8s1 (1 Oct): reopening a case is explicit: a note on an answered or picked-up case says `reopen`, and the writer refuses one that does not, as a transition.
- `note at the case's revision is accepted`: the note on the answered case says `reopen`. Cases 6M6gx and AV8s1 (1 Oct): reopening a case is explicit: a note on an answered or picked-up case says `reopen`, and the writer refuses one that does not, as a transition.

### Moved here

- `an append keeps a time in year 1` (was `an append whose record has the zero time is stamped with now`), from `go-quirks.json`: stamped with now before; now the record keeps its time. ST-06: there is no zero time.

## `freshness.json`

New: a core vector file. Each vector draws a form from a case, by `drawn_after` or `drawn_at`, and expects `since`, how the case changed since. One rule in the core for both inboxes, in place of a rule each wrote on its own (#187, #188).

### Added

- `nothing since the form was drawn: unchanged`
- `a note on an open case: unchanged`
- `several notes on an open case: unchanged`
- `rows numbered with gaps, and a note since: unchanged`
- `an amend of links on an open case: unchanged`
- `an amend of labels and context on an open case: unchanged`
- `an amend of the recommend on an open case: unchanged`
- `an amend of the title on an open case: unchanged`
- `a briefing's amend of landed: unchanged`
- `an amend that sets the body the case has: unchanged`
- `an amend of the body on an open case changes the question: changed`
- `an amend that adds an option changes the question: changed`
- `an amend of the question, then a note: changed`
- `an amend of the question before the form was drawn, then a note: unchanged`
- `the human's edit of a sent message's body: unchanged`
- `the human's edit of a sent message, then a pickup: moved`
- `a message picked up: moved`
- `a message withdrawn by the human: moved`
- `answered elsewhere: moved`
- `a note, then an answer: moved`
- `two notes, then an answer: moved`
- `an answer picked up: moved`
- `answered, picked up and closed: moved`
- `withdrawn by the agent: moved`
- `an answered case withdrawn by the human: moved`
- `parked: moved`
- `a reopening note on an answered case: moved`
- `a reopening amend of links on an answered case: moved`
- `a reopening amend that changes the question: changed`
- `answered elsewhere and reopened between two reads: changed`
- `answered, reopened, answered again and picked up: changed`
- `parked and resumed: changed`
- `a row the fold skipped since: changed`
- `a row the fold skipped before the form was drawn, then a note: unchanged`
- `a revision above the case's: changed`
- `a form drawn before the open: changed`
- `a seq above the case's: changed`
- `a seq below the one its revision had, then a note: changed`

## `stats.json`

New: a core vector file, of its own shape (README, "Stats"). Each vector gives the rows of some cases, a clock and a number of days, and expects the cases opened and answered on each day and the median time to answer. The TUI's stats strip shows them, worked out from the cases it has read, so they need no store call.

### Added

- `no cases: every day is 0, and there is no median`
- `no days: the lists are empty`
- `a case opened today and not answered`
- `two cases opened on one day count 2`
- `a case opened at midnight UTC counts on the day that begins`
- `a case opened just before midnight UTC counts on the day that ends`
- `a time with an offset counts on its day in UTC`
- `a case opened later today than now counts today`
- `a case opened tomorrow, by a clock ahead, is outside the days`
- `a case opened the day before the first day is outside the days`
- `now at midnight: today is the day that begins at now`
- `fourteen days: the first is 13 days before today`
- `a case answered on a later day than it opened counts on each day`
- `a case answered the day it opened counts both on that day`
- `a case opened before the days and answered in them counts its answer and its time to answer`
- `an answer before the days is not counted, nor its time to answer`
- `an answer after today, by a clock ahead, is not counted`
- `a case answered, reopened by a note and answered again counts both answers, the second timed from the reopen`
- `a case picked up and reopened by an amend: the second answer is timed from the amend`
- `a case reopened and not answered again counts its one answer`
- `a reopen without a time leaves the next answer with no time to answer`
- `a note on an open case is not a reopen: the answer is timed from the open`
- `a case parked and resumed is timed from its open`
- `the median of an odd count is the middle one`
- `the median of an even count is the mean of the middle two`
- `the median is cut to whole seconds`
- `a time to answer counts its fraction of a second`
- `the median is over answers on every day counted`
- `a drop is not an answer`
- `a withdrawn case counts as opened and not answered`
- `a message is not counted`
- `an answer to each kind the human answers counts`
- `an answer without a time is not counted`
- `a case opened with no time is not counted as opened, and its answer has no time to answer`
- `an answer before its open, by a clock behind, counts with no time to answer`
- `an answer with a time to answer of zero counts it`

## `go-quirks.json`

### Moved away

- `an answer that did not see an amend is accepted when it sends no revision`: to `answer.json` as `amended: an answer that did not see the amend is refused`.
- `a response field at its zero value is not set: other false beside a choice`: to `answer.json`.
- `a response field at its zero value is not set: ack false beside drop`: to `answer.json`.
- `a response field at its zero value is not set: no rows beside text`: to `answer.json`.
- `an open without a kind is refused as unknown`: to `open.json` as `an open without a kind is refused`.
- `an open without an urgency is refused as unknown`: to `open.json` as `an open without an urgency is refused`.
- `an open may give an option twice, which an amend may not`: to `open.json` as `an option given twice is refused`.
- `an open may have a link of only white space, which an amend may not`: to `open.json` as `a link of only white space is refused`.
- `an open may give a link twice, which an amend may not`: to `open.json` as `a link given twice is refused`.
- `an appended open is checked for its own rules before its actor`: to `open.json` as `an appended open is checked for its actor before its own rules`.
- `a stored open is checked for its actor before its own rules`: to `open.json`.
- `field names match whatever their case`: to `fold.json` as `field names are matched exactly: Kind is not kind, and ACK is not ack`.
- `fields whose names differ only in case are one field, and the last one wins`: to `fold.json` as `fields whose names differ only in case are two fields, not one given twice`.
- `when a field is given twice the last one wins`: to `fold.json` as `a key given twice is malformed`.
- `a record of null reads as an empty record`: to `fold.json` as `a record of null is malformed`.
- `a field of null reads as a field not given`: to `fold.json` as `a field of null is malformed`.
- `a title of null is a missing title, which breaks the open's rules rather than being malformed`: to `fold.json` as `a title of null is malformed, not a missing title`.
- `a response field of null is not set`: to `fold.json` as `a response field of null is malformed`.
- `a lower-case z, which RFC 3339 allows, is malformed`: to `fold.json` as `a lower-case z, which RFC 3339 allows, is read`.
- `a lower-case t, which RFC 3339 allows, is malformed`: to `fold.json` as `a lower-case t, which RFC 3339 allows, is read`.
- `a time written with a JSON escape is malformed, though the string is the same`: to `fold.json` as `a time written with a JSON escape is read as the string it spells`.
- `a leap second, which RFC 3339 allows, is malformed`: to `fold.json` as `a leap second is malformed, though RFC 3339 allows one at the end of some days`.
- `an hour of one digit, which RFC 3339 does not allow, is read`: to `fold.json` as `an hour of one digit, which RFC 3339 does not allow, is malformed`.
- `a comma before the fraction, which RFC 3339 does not allow, is read`: to `fold.json` as `a comma before the fraction, which RFC 3339 does not allow, is malformed`.
- `an offset of +24:00, which RFC 3339 does not allow, is read`: to `fold.json` as `an offset of +24:00, which RFC 3339 does not allow, is malformed`.
- `an offset with a minute of 60, which RFC 3339 does not allow, is read`: to `fold.json` as `an offset with a minute of 60, which RFC 3339 does not allow, is malformed`.
- `a record without its time folds, with no time`: to `fold.json`.
- `an event without a time leaves updated_at as it was`: to `fold.json`.
- `the zero time reads as no time`: to `fold.json` as `a time in year 1 is a time like any other`.
- `an append whose record has the zero time is stamped with now`: to `writer.json` as `an append keeps a time in year 1`.

### Removed

- `a stored answer of only ack false is no response`: 'fyi: ack false is no response' in answer.json has the rule. A field at its zero value reads as a field left out (case 2026-09-24T20-38-25Z-does-a-zero-value-field-count-as-given).
- `field names match by Unicode simple case folding, so a Kelvin sign reads as k`: no name is folded, so a Kelvin sign is one more name that is not kind, as Kind is in 'field names are matched exactly: Kind is not kind, and ACK is not ack'. ST-06: field names match exactly.
- `a field given twice, the second time as null, keeps the first value`: The rules for merging a key given twice are gone; 'a key given twice is malformed' covers it. ST-06: a key given twice is malformed.
- `an object given twice is merged: the second gives only what it names`: The rules for merging a key given twice are gone; 'a key given twice is malformed' covers it. ST-06: a key given twice is malformed.
- `an object given twice, the second time as null, is cleared`: The rules for merging a key given twice are gone; 'a key given twice is malformed' covers it. ST-06: a key given twice is malformed.
- `a list given twice, the second time as null, is cleared`: The rules for merging a key given twice are gone; 'a key given twice is malformed' covers it. ST-06: a key given twice is malformed.
- `a list given twice is replaced, but an object in it is merged into the one at the same place`: The rules for merging a key given twice are gone; 'a key given twice is malformed' covers it. ST-06: a key given twice is malformed.
- `the same answer is refused as stale when it sends the revision it read`: AG-12: the revision guard is unchanged; 'stale is checked before the record's rules' in writer.json has the same rows and at_revision.
- `an answer from before an amend that only added labels is refused as stale too`: AG-12, ST-04: 'an append at an earlier revision is refused as stale' in writer.json shows the same: its amend (a context) leaves amend_seq alone, and the revision still refuses the answer.

## `store/version.json`

New: a store vector file, which the core's tests run against a store in memory and `cases-store-sqlite`'s against a SQLite file. ST-16, ST-13 and the hosted plan's C2: the version moves on every event, archive and delete, and never on listening or a refused write (case `rx6Cl`).

### Added

- `the version moves on an open, each event, an archive and a delete`
- `an open after a delete moves the version, and so does the next delete`
- `a refused event, archive or delete leaves the version`
- `a refused open leaves the version and opens no case`
- `an archive or a delete of a case no longer there is refused, and leaves the version`
- `listening, a heartbeat and doing lines leave the version`

## `store/changes.json`

New: a store vector file, which the core's tests run against a store in memory and `cases-store-sqlite`'s against a SQLite file. The hosted plan's C1 and C2: rows_since's three answers, as spec/api-agents.md's `?since=V` gives them (case `k8yKT`).

### Added

- `with no version, every case`
- `since the version now, nothing changed`
- `since an earlier version, the case with a row stored since`
- `a case opened since is changed`
- `since the store was empty, every case opened is changed`
- `cases archived or deleted since are removed`
- `a case opened and removed since may be named as removed`
- `a refused write changes nothing since`
- `a case changed and then archived is removed, not changed`

## `store/actors.json`

New: a store vector file, which the core's tests run against a store in memory and `cases-store-sqlite`'s against a SQLite file. ST-12: one actor for each writer, by kind and name.

### Added

- `one actor for each writer, however many times it writes`
- `an actor for each name and kind, and one unnamed for each kind, in the order made`
- `an agent and a human of one name are two actors`
- `a refused write makes no actor`

## `store/listening.json`

New: a store vector file, which the core's tests run against a store in memory and `cases-store-sqlite`'s against a SQLite file. ST-16 and AG-17: the local listening record and its lapse, and the doing line's rule, which moved into the core with these vectors.

### Added

- `a listener is listed with its labels and times`
- `a listener never makes the store`
- `a blank worker is refused, and records nothing`
- `a blank label is left out`
- `two waits of one worker are one listener, on the labels of both`
- `a heartbeat keeps when the wait began`
- `a new wait of a worker listening takes when its worker began`
- `a listener lapses 45 seconds after it was last heard`
- `a listener heard later than now, by a clock ahead, is listening`
- `a worker that listens again after it lapsed starts afresh`
- `a doing line keeps its time until it changes, and blank clears it`
- `a heartbeat keeps the doing line`
- `a doing line needs a listening worker`
- `a doing line is trimmed`
- `a doing line of 80 characters is taken`
- `a doing line of 81 characters is refused, before whether the worker listens`
- `a doing line with a line break or another control character is refused, before whether the worker listens`
- `a doing line with a character that sets the direction of the text after it is refused, before whether the worker listens`
- `a character just outside the ranges that set direction is taken`
- `a listen with a doing line that is not one records nothing`
- `a refused open does not make the store, so a listener is still refused`
