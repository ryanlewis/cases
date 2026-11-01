# Parity with the Go CLI

Every test of the Go CLI, `cmd/cases` in `ryanlewis/cases` at 508548b, and what covers it here. Each one is:

- **covered**: a test here checks the same behaviour;
- **changed**: v1 changes the behaviour, for the story or decision named, and a test here checks the new behaviour;
- **dropped**: the test is of Go-only code, for the reason given;
- **pending**: the command it tests is not written yet. The worker that writes it fills the row in.

The stories in `~/dev/notes/projects/cases/spec/stories.md` are the contract where they differ from Go. Output text is not: a test here pins text only where people or scripts read it, such as the table's columns.

Tests here are named by file and function: `tests/show.rs` is an integration test, which runs the built binary against a store and a home of its own, and `config::tests` a unit test in `src/config.rs`. Where a Go test ran a command that is not written yet to set a case up, such as `answer`, the test here writes the event through the store.

Tally: 158 Go tests, 120 covered, 36 changed, 2 dropped, 0 pending.

## `amend_test.go`

| Go test | Status | Rust test |
| --- | --- | --- |
| TestAmendRoundTrip | covered | `tests/amend.rs` `amend_adds_to_the_case_and_replaces_its_body_and_context` |
| TestShowPrintsWhatAnAmendReplaced | covered | `tests/show.rs` `show_prints_what_an_amend_replaced` |
| TestAmendAtRevision | changed, because CL-01 and AG-12: a stale write exits 4, and the error names the state the case is in now | `tests/amend.rs` `amend_at_a_revision_the_case_has_left_is_refused_as_stale` |
| TestAmendInlineBody | covered | `tests/amend.rs` `amend_takes_an_inline_body` |
| TestAmendAddsOptions | covered | `tests/amend.rs` `amend_adds_options_that_an_answer_can_choose` |
| TestAmendAddsLabels | covered | `tests/amend.rs` `amend_adds_labels` |
| TestAmendRefusals | changed, because ST-10: an id that is a path names no case, like any other text. Refusals are compared by exit code and an untouched case, not by message | `tests/amend.rs` `a_refused_amend_writes_nothing` |

## `answer_test.go`

| Go test | Status | Rust test |
| --- | --- | --- |
| TestAnswerByKind | changed, because HU-03 and HU-02: park is its own command, so `answer --park` is refused, and drop is checked on every kind | `tests/answer.rs` `answer_takes_the_response_each_kind_asks_for`, `an_answer_that_does_not_fit_the_kind_is_refused`; `tests/park.rs` `park_and_resume_a_stuck_case`, `only_a_stuck_case_parks` |
| TestAnswerTextFile | covered | `tests/answer.rs` `answer_reads_its_text_from_a_file_or_stdin` |
| TestAnswerAtRevision | changed, because CL-01 and AG-12: a stale write exits 4, and the error names the state now | `tests/answer.rs` `answer_at_a_revision_the_case_has_left_is_refused_as_stale` |
| TestAnswerParkAtRevision | changed, because HU-03: park is its own command | `tests/park.rs` `park_and_resume_at_a_revision_the_case_has_left_are_refused_as_stale` |
| TestAnswerTakesPartOfAnID | changed, because ST-10: the part is of the opaque id | `tests/answer.rs` `answer_takes_part_of_an_id_and_agent_commands_do_not` |
| TestNameConfigStampsTheHumanOnAnswer | changed, because HU-06 and ST-12: each event holds an actor id, named in the actors table; the human's name is `--as`, then the config file's `name`, then the login name; and an agent event on a case with no worker carries the unnamed agent | `tests/answer.rs` `the_human_is_named_by_as_then_the_config_file_then_the_login_name`; `tests/agent.rs` `the_agents_writes_carry_the_workers_actor` |
| TestBlankWorkerRecordsNoActor | changed, because AG-02 and ST-12: open refuses a blank worker, and the agent's events on a case with no worker carry the unnamed agent | `tests/open.rs` `a_refused_open_writes_nothing_and_exits_1`; `tests/agent.rs` `the_agents_writes_carry_the_workers_actor` |

## `close_test.go`

| Go test | Status | Rust test |
| --- | --- | --- |
| TestClose | covered | `tests/agent.rs` `close_records_the_outcome_of_a_picked_up_case` |
| TestCloseAtRevision | changed, because CL-01 and AG-12: a stale write exits 4 | `tests/agent.rs` `close_at_a_revision_the_case_has_left_is_refused_as_stale` |
| TestCloseInlineOutcome | covered | `tests/agent.rs` `close_takes_an_inline_outcome` |

## `config_test.go`

| Go test | Status | Rust test |
| --- | --- | --- |
| TestConfigKeysMatchFlags | covered: `store`, `name` (`--as` on answer, park, resume, sweep and serve), `prune-age` (prune's `--age`), and `listen` and `no-open` (serve's) have their flags. Each key is read and checked | `config::tests` `every_key_reads`; `tests/answer.rs` `the_human_is_named_by_as_then_the_config_file_then_the_login_name`; `tests/prune.rs` `the_config_file_sets_the_age_below_the_flag`; `tests/serve.rs` `the_config_file_gives_the_address_and_the_flag_wins`; `commands::serve::tests` `the_no_open_flag_wins_over_the_config_file` |
| TestConfigSetsStore | covered | `tests/global.rs` `the_config_file_names_the_store_below_the_flag_and_the_environment` |
| TestConfigExpandsHome | covered, for `--store` too | `tests/global.rs` `a_leading_tilde_is_the_home_directory`, `config::tests` `a_leading_tilde_is_the_home_directory` |
| TestConfigSetsListen | covered, for serve's `--listen` and `service install`'s; an address from the file is checked as the flag's is | `tests/serve.rs` `the_config_file_gives_the_address_and_the_flag_wins`; `tests/service.rs` `install_takes_the_address_and_name_from_the_config_file` |
| TestConfigSetsNoOpen | covered: the key reads as a boolean or as `"true"` or `"false"`, and `--no-open` and `--no-open=false` win over it. That serve then leaves the browser alone shows only on a terminal, which no test has | `config::tests` `every_key_reads`, `a_key_of_the_wrong_type_is_refused`; `commands::serve::tests` `the_no_open_flag_wins_over_the_config_file` |
| TestConfigFlagAndEnv | covered | `tests/global.rs` `the_config_file_comes_from_the_flag_or_the_environment`; `tests/config.rs` `config_path_names_the_file_and_whether_it_is_there` |
| TestConfigFlagLastWins | covered | `tests/global.rs` `the_last_config_flag_wins` |
| TestBrokenConfigStopsCommandsButNotDiagnosis | changed, because SV-01: `config show` also prints the file as it is (case 2026-09-25T20-55-32Z-broken-config-what-do-config-show-and-init-do) | `tests/global.rs` `a_config_file_that_cannot_be_used_stops_every_command_but_config`, `a_config_file_of_the_wrong_shape_is_named_in_the_error`; `tests/config.rs` `a_config_file_that_cannot_be_used_is_still_reported`; `config::tests` `a_key_of_the_wrong_type_is_refused`, `a_toml_error_names_its_line_and_column`, `a_directory_cannot_be_read` |
| TestConfigShow | covered | `tests/config.rs` `config_show_prints_each_default_and_where_it_came_from` |
| TestConfigInit | covered; the template is checked to name every key and set none | `tests/config.rs` `config_init_writes_a_template_and_refuses_to_overwrite_unless_forced`; `commands::config::tests` `the_template_names_every_key_and_sets_none` |

## `e2e_test.go`

| Go test | Status | Rust test |
| --- | --- | --- |
| TestDecisionEndToEnd | covered | `tests/wait.rs` `a_decision_goes_from_open_to_closed` |

## `inbox_test.go`

| Go test | Status | Rust test |
| --- | --- | --- |
| TestInboxPrintsTheRunningURL | covered | `tests/inbox.rs` `inbox_print_prints_the_running_url` |
| TestInboxPrintsACasesPage | covered, with part of an id and an id with a slug too | `tests/inbox.rs` `inbox_print_prints_a_case_s_page` |
| TestInboxPrintUnknownCase | covered | `tests/inbox.rs` `inbox_print_refuses_an_unknown_case` |
| TestInboxOpensTheBrowserWhenNotPrinting | covered, with a stand-in `open` and `xdg-open` first on `$PATH` in place of Go's injected opener; with no `$PATH` the opener is not looked for in `/usr/bin` | `tests/inbox.rs` `inbox_opens_the_browser_when_not_printing`, `inbox_without_a_browser_opener_fails` |
| TestInboxWithoutServe | covered | `tests/inbox.rs` `inbox_without_serve_says_how_to_start_one` |

## `list_test.go`

| Go test | Status | Rust test |
| --- | --- | --- |
| TestListOrderFilterAndJSON | changed, because AG-14: within an urgency, cases list in the order they were opened, and each `--json` item has its revision | `tests/list.rs` `list_sorts_by_urgency_then_open_order`, `list_filters_by_state_and_prints_json` |
| TestListShowsLabels | covered | `tests/list.rs` `list_shows_labels_before_the_title` |
| TestListByLabelAndWorker | covered | `tests/list.rs` `list_filters_by_label_and_worker` |
| TestListReportsBrokenCaseAndListsTheRest | covered | `tests/list.rs` `list_warns_about_a_case_that_does_not_fold_and_lists_the_rest` |
| TestListDefaultsToOpenAndParked | covered | `tests/list.rs` `list_defaults_to_open_and_parked` |
| TestListByKindAndUrgency | covered | `tests/list.rs` `list_filters_by_kind_and_urgency`, `tests/sweep.rs` `sweep_filters_by_label_worker_kind_and_the_age_of_the_last_event`, `tests/wait.rs` `wait_by_kind` |
| TestWaitByKind | covered | `tests/wait.rs` `wait_by_kind` |
| TestListCount | covered | `tests/list.rs` `list_count_prints_only_the_number` |
| TestListOlderThanReadsTheLastEvent | changed: `--older-than 0` takes every case, one whose last event is dated in the future too, as `prune --age 0` does (ST-13); Go's took none dated in the future | `tests/list.rs` `list_older_than_reads_the_last_event` |

## `main_test.go`

| Go test | Status | Rust test |
| --- | --- | --- |
| TestStoreDefaultsToDataHome | covered | `tests/global.rs` `the_store_defaults_to_the_data_home`; `tests/config.rs` `config_show_gives_the_store_under_the_data_home_by_default` |
| TestListOnMissingStoreIsEmpty | covered, and list does not make the store | `tests/list.rs` `list_on_a_store_that_is_not_there_is_empty` |
| TestStoreFromEnvironment | covered | `tests/global.rs` `the_store_comes_from_the_environment` |
| TestVersionString | covered | `tests/global.rs` `version_prints_the_version` |
| TestReportExitStatus | changed, because CL-01: a stale write exits 4, not 1. A pickup of an open case exits 3, an unknown case 1, and a `wait` that times out 2 with no `Error: ` line | `tests/agent.rs` `pickup_records_who_read_the_answer`; `tests/answer.rs` `answer_at_a_revision_the_case_has_left_is_refused_as_stale`; `tests/show.rs` `show_refuses_an_id_that_names_no_case`; `error::tests` `a_refused_transition_exits_3`, `a_stale_write_exits_4`, `every_other_error_exits_1`; `tests/global.rs` `bad_input_exits_1_not_clap_s_2`; `tests/wait.rs` `wait_times_out_with_exit_2_and_nothing_on_stdout` |
| TestFindCaseTakesPartOfAnID | changed, because ST-10 and HU-07: ids are opaque, so a part of the id is matched, not a slug; the several matches are listed with their titles; a whole id not in the store matches no other case; and there are no same-second siblings | `tests/show.rs` `show_takes_part_of_an_id`, `show_lists_the_cases_a_part_of_an_id_matches`, `a_whole_id_that_is_not_in_the_store_matches_no_other_case`, `show_on_a_store_that_is_not_there` |

## `note_test.go`

| Go test | Status | Rust test |
| --- | --- | --- |
| TestNoteReopensAnAnsweredCase | covered | `tests/agent.rs` `a_note_reopens_an_answered_case` |
| TestNoteAtRevision | changed, because CL-01 and AG-12: a stale write exits 4 | `tests/agent.rs` `a_note_at_a_revision_the_case_has_left_is_refused_as_stale` |
| TestNoteInlineBody | covered | `tests/agent.rs` `a_note_takes_an_inline_body` |

## `open_test.go`

| Go test | Status | Rust test |
| --- | --- | --- |
| TestOpenDecisionKeepsCommasInOptions | changed, because ST-10: the id is the key's encoding, with no slug | `tests/open.rs` `open_keeps_commas_in_options` |
| TestOpenBody | covered | `tests/open.rs` `open_reads_the_body_from_a_file_or_stdin` |
| TestOpenInlineBody | covered | `tests/open.rs` `open_takes_an_inline_body` |
| TestOpenApprovalRows | covered | `tests/open.rs` `open_writes_approval_rows` |
| TestOpenLabels | covered | `tests/open.rs` `open_writes_labels_and_the_brief` |
| TestOpenWorkerAndLabelFromTheEnvironment | changed, because AG-01: an empty or blank CASES_LABEL or CASES_WORKER counts as not given, where Go refused an empty label (case 2026-09-25T20-55-32Z-empty-cases-label-not-given-or-refused) | `tests/open.rs` `open_takes_the_worker_and_one_label_from_the_environment` |
| TestOpenRefusals | changed, because AG-02: open also refuses an option or link given twice, a blank link, and a worker, brief, context or `for` of only white space. Refusals are compared by exit code and an untouched store, not by message | `tests/open.rs` `a_refused_open_writes_nothing_and_exits_1`, `an_inline_body_that_names_a_file_says_to_use_the_file_flag`, `a_whitespace_body_is_accepted`; `open::tests` `a_row_that_is_not_one_object_is_refused` |

## `pickup_test.go`

| Go test | Status | Rust test |
| --- | --- | --- |
| TestPickupAtRevision | changed, because CL-01 and AG-12: a stale write exits 4 | `tests/agent.rs` `pickup_at_a_revision_the_case_has_left_is_refused_as_stale` |
| TestPickup | covered; a pickup of an open case exits 3 | `tests/agent.rs` `pickup_records_who_read_the_answer` |

## `prune_test.go`

| Go test | Status | Rust test |
| --- | --- | --- |
| TestPruneDryRunWritesNothing | covered | `tests/prune.rs` `a_dry_run_writes_nothing` |
| TestPruneArchives | covered, and a case opened after the prune gets a key of its own (ST-10) | `tests/prune.rs` `prune_moves_each_case_whole_into_the_archive` |
| TestPruneRefusesAnIDInTheArchive | covered | `tests/prune.rs` `a_case_whose_key_the_archive_has_is_left` |
| TestPruneDelete | covered | `tests/prune.rs` `prune_delete_removes_the_cases` |
| TestPruneSkipsABrokenCase | changed, because ST-13: prune still prunes the rest, then names the case it could not load and exits 1 (case 2026-09-25T20-55-32Z-prune-leaving-a-case-it-can-t-load-exit-1-or-0) | `tests/prune.rs` `prune_leaves_a_case_that_does_not_fold_names_it_and_exits_1` |
| TestPruneRefusesOtherStates | covered | `tests/prune.rs` `prune_takes_only_closed_and_withdrawn_cases` |
| TestPruneOnMissingStore | covered | `tests/prune.rs` `prune_on_a_store_that_is_not_there_does_nothing` |
| TestConfigSetsPruneAge | covered | `tests/prune.rs` `the_config_file_sets_the_age_below_the_flag` |
| TestPruneAgeZeroTakesAFutureCase | covered | `tests/prune.rs` `an_age_of_0_takes_a_case_dated_in_the_future` |

## `resume_test.go`

| Go test | Status | Rust test |
| --- | --- | --- |
| TestResume | changed, because HU-05 and HU-03: resume is the human's alone, with no `--agent`, and the park is its own command | `tests/park.rs` `park_and_resume_a_stuck_case`, `resume_takes_no_agent_flag` |
| TestResumeAtRevision | changed, because HU-05 and CL-01: the resume between is the human's, and a stale write exits 4 | `tests/park.rs` `park_and_resume_at_a_revision_the_case_has_left_are_refused_as_stale` |
| TestResumeTakesPartOfAnID | changed, because ST-10: the part is of the opaque id; park takes one too | `tests/park.rs` `park_and_resume_take_part_of_an_id` |

## `serve_screen_test.go`

| Go test | Status | Rust test |
| --- | --- | --- |
| TestScreenDrawRedrawsInPlace | covered: a frame clears each line, moves up over the last one, and is not drawn again when it is the same; the counts and the log are laid out | `commands::serve::screen::tests` `a_frame_draws_over_the_last_and_only_when_it_changed`, `the_screen_counts_the_store`, `an_empty_store_and_a_failed_read` |
| TestScreenDrawStoreStates | covered: a store not made yet reads as empty, and one that cannot be read is said in red | `commands::serve::watch::tests` `a_missing_store_reads_as_empty_and_its_first_case_notifies`; `commands::serve::screen::tests` `an_empty_store_and_a_failed_read` |
| TestScreenLogWriter | covered for keeping the last five lines. The Rust log is given whole lines, so the part of the Go test about a line split across writes has nothing to test | `commands::serve::log::tests` `the_screen_keeps_the_last_lines` |
| TestScreenLogWriterWithoutTee | covered | `commands::serve::log::tests` `the_screen_keeps_the_last_lines` |
| TestScreenLogWriterStderrFails | dropped: Go's log was a writer whose failed write the server saw. The Rust log writes a line to stderr and keeps it for the screen whatever stderr does, so there is no failure to pass on |  |
| TestScreenRunWithoutTerminal | covered for what a frame shows without keys to read (no `q`). The drawing loop, with its hidden cursor and wrapping, runs only when stdout is a terminal, which no test has | `commands::serve::screen::tests` `an_empty_store_and_a_failed_read`, `a_frame_draws_over_the_last_and_only_when_it_changed` |
| TestReadKeys | covered | `commands::serve::term::tests` `q_stops_serve_and_other_keys_do_not` |
| TestUptimeAndAgo | covered | `commands::serve::screen::tests` `uptime_and_ages_read_at_a_glance` |

## `serve_test.go`

| Go test | Status | Rust test |
| --- | --- | --- |
| TestServeRefusesNonLoopback | covered | `tests/serve.rs` `serve_refuses_an_address_that_is_not_loopback` |
| TestServeAnswersAndStops | changed, because IN-02 and SV-02: the page reads the case from the API (spec/api.md), so the test reads and answers it there, and sees the write logged and serve stop | `tests/api.rs` `a_case_is_what_show_json_gives`, `an_answer_writes_one_event_as_serves_human`; `tests/serve.rs` `serve_logs_writes_and_failures_but_not_a_page_s_own_reads`, `serve_records_itself_for_status_and_removes_the_record_when_it_stops` |
| TestServePushesChangesAndStopsWithAStreamOpen | covered, and the stream ends cleanly | `tests/serve.rs` `serve_pushes_changes_and_stops_with_a_stream_open` |
| TestServeWithoutTerminalPrintsURLAndOpensNothing | covered, with a stand-in opener on `$PATH` that is never run | `tests/serve.rs` `off_a_terminal_serve_prints_its_address_and_opens_nothing` |
| TestBrowserCommand | changed: the opener is `open` on macOS and `xdg-open` elsewhere, found on `$PATH` only and given the URL as one argument; there is no Windows opener | `tests/inbox.rs` `inbox_opens_the_browser_when_not_printing`, `inbox_without_a_browser_opener_fails` |
| TestServeScreenStats | changed: the screen counts the cases closed since serve started, where Go counted those closed today, as serve knows no time zone | `commands::serve::screen::tests` `the_screen_counts_the_store` |
| TestServeFailsWhenAddressIsTaken | covered | `tests/serve.rs` `serve_fails_when_its_address_is_taken` |
| TestServeWithoutStateDirectoryStillServes | covered | `tests/serve.rs` `serve_runs_without_a_state_directory_it_can_write` |
| TestServeQueuesNotificationsWithNoTabOpen | covered, and serve reads the store before it says it is up, so a case opened as soon as it is up notifies | `tests/serve.rs` `serve_queues_notifications_with_no_page_open`, `a_case_opened_as_soon_as_serve_is_up_notifies` |

## `serve_unix_test.go`

| Go test | Status | Rust test |
| --- | --- | --- |
| TestServeStopsOnSignal | covered, for SIGTERM and SIGINT (Ctrl-C). A second signal ends a slow stop at once, as Go's serve restored the default after the first | `tests/serve.rs` `serve_records_itself_for_status_and_removes_the_record_when_it_stops`, `serve_stops_on_ctrl_c_and_removes_its_record`, `a_second_ctrl_c_ends_a_slow_stop_at_once` |

## `service_test.go`

| Go test | Status | Rust test |
| --- | --- | --- |
| TestServiceInstallWritesUnit | covered: the command on Linux, and the unit and its commands on every system | `tests/service.rs` `install_writes_the_service_and_starts_it`; `service::tests` `the_unit_restarts_serve_and_quotes_what_systemd_reads`, `install_writes_the_unit_then_reloads_enables_and_restarts_it` |
| TestServiceInstallWritesPlist | covered: the command on macOS, and the plist and its commands on every system | `tests/service.rs` `install_writes_the_service_and_starts_it`; `service::tests` `the_plist_runs_serve_at_load_and_keeps_it_alive`, `install_writes_the_plist_and_bootstraps_it` |
| TestServiceInstallTakesConfig | covered, with the flags beating the file | `tests/service.rs` `install_takes_the_address_and_name_from_the_config_file` |
| TestServiceInstallRefusesNonLoopback | covered, from the flag and from the config file | `tests/service.rs` `install_refuses_an_address_that_is_not_loopback`; `listen::tests` |
| TestServiceUninstall | covered | `tests/service.rs` `uninstall_stops_the_service_and_removes_its_file`; `service::tests` `uninstall_boots_out_the_agent_and_removes_the_plist`, `uninstall_disables_the_unit_and_removes_it` |
| TestServiceManagerDown | covered | `tests/service.rs` `a_service_manager_that_cannot_be_asked_changes_nothing`; `service::tests` `a_query_that_fails_stops_install_and_uninstall_before_they_change_the_file` |
| TestServiceInstallWithoutManager | covered: the tests reach no real service manager because `launchctl` and `systemctl` are looked for on `$PATH` only, where the tests put stand-ins; without them every service command fails and writes nothing | `tests/service.rs` `without_the_service_manager_on_path_nothing_is_touched` |
| TestServiceInstallRefusesWhileServeRuns | covered | `tests/service.rs` `install_refuses_while_a_serve_runs_for_the_store` |
| TestServiceInstallRefusesTakenAddress | covered | `tests/service.rs` `install_refuses_an_address_that_is_taken` |
| TestServiceReinstallWhileServiceRuns | covered | `tests/service.rs` `reinstall_while_the_service_runs` |
| TestServiceReinstallChecksWhatChanged | covered | `tests/service.rs` `reinstall_checks_what_changed` |
| TestServiceStatusNotInstalled | covered | `tests/service.rs` `service_status_with_nothing_installed` |
| TestServiceStatusHandStartedServe | covered | `tests/service.rs` `service_status_names_a_serve_started_by_hand` |
| TestServiceStatusRunning | covered | `tests/service.rs` `service_status_when_installed_loaded_and_answering` |
| TestServiceStatusNotLoaded | covered | `tests/service.rs` `service_status_when_installed_but_not_loaded` |
| TestServiceStatusNotAnswering | covered | `tests/service.rs` `service_status_when_loaded_but_not_answering` |
| TestServiceStatusLoadedWithoutFile | covered | `tests/service.rs` `service_status_when_loaded_with_its_file_gone` |
| TestServiceStatusOtherStoreAndBinary | covered | `tests/service.rs` `service_status_names_another_store_and_another_binary` |

## `show_test.go`

| Go test | Status | Rust test |
| --- | --- | --- |
| TestShow | changed, because AG-13: each actor is named from the actors table, and the thread says who wrote each event. The answer is written through the store until `answer` is written | `tests/show.rs` `show_prints_the_case_and_its_thread` |
| TestShowQuestionReply | covered | `tests/show.rs` `show_describes_a_question_reply_as_a_reply` |
| TestShowRefusesPathsAndMissingCases | changed, because ST-10: a path is text that names no case, like any other | `tests/show.rs` `show_refuses_an_id_that_names_no_case` |
| TestShowJSONRevisionCountsSkippedFiles | covered; the next write goes through the store until `answer` is written | `tests/show.rs` `show_json_revision_counts_skipped_rows` |
| TestShowTakesPartOfAnID | changed, because ST-10: the part is of the opaque id, and a title slug after an id is ignored | `tests/show.rs` `show_takes_part_of_an_id` |
| TestShowReportsAnUnknownEventAsVersionSkew | covered, without Go's `go install` advice | `tests/show.rs` `show_reports_an_unknown_event_as_version_skew` |
| TestShowURLOfTheRunningInbox | covered, with a stand-in instance file and against a real serve | `tests/show.rs` `show_gives_the_url_of_the_running_inbox`; `tests/api.rs` `a_case_is_what_show_json_gives` |
| TestShowWarnsOnADamagedInstanceFile | covered | `tests/show.rs` `show_warns_on_a_damaged_instance_file` |
| TestShowAnswer | covered; the answer, pickup and note go through the store until those commands are written | `tests/show.rs` `show_answer_prints_only_the_state_kind_revision_and_answer` |

## `skill_test.go`

| Go test | Status | Rust test |
| --- | --- | --- |
| TestSkillShowPrintsFrontmatter | covered | `tests/skill.rs` `skill_show_prints_the_skill_with_its_frontmatter`; `commands::skill::tests` `skill_md_starts_with_frontmatter_that_names_it` |
| TestSkillInstallListUninstall | covered, with a file beside the skill left on uninstall | `tests/skill.rs` `skill_install_list_and_uninstall`, `each_agent_has_its_own_directory` |
| TestSkillCheck | covered | `tests/skill.rs` `skill_check_fails_only_on_a_stale_or_unreadable_skill` |
| TestSkillUnreadable | covered | `tests/skill.rs` `an_unreadable_skill_is_reported_and_replaced_only_with_yes` |
| TestSkillUnknownAgent | covered: clap refuses it and names every agent | `tests/skill.rs` `an_unknown_agent_is_refused_and_every_agent_is_named` |
| TestSkillHelpNamesEveryAgent | covered: each agent argument offers the agents clap checks it against, so the help cannot miss one | `commands::skill::tests` `each_agent_argument_offers_every_agent` |

## `status_test.go`

| Go test | Status | Rust test |
| --- | --- | --- |
| TestServeRecordsInstanceForStatus | covered, with a stand-in instance file and against a real serve, which removes its file when it stops and records its store as Go's `filepath.Abs` gave it | `tests/status.rs` `status_prints_the_running_serve`; `tests/serve.rs` `serve_records_itself_for_status_and_removes_the_record_when_it_stops`, `serve_records_its_store_without_dot_dots` |
| TestStatusWithoutServe | covered | `tests/status.rs` `status_without_serve_is_not_running` |
| TestStaleInstanceIsNotRunning | covered, and a fresh serve replaces the stale file | `tests/status.rs` `a_stale_instance_file_is_not_running`; `tests/serve.rs` `a_stale_record_is_replaced` |
| TestSecondServeOnSameStoreIsRefused | covered | `tests/serve.rs` `a_second_serve_on_the_same_store_is_refused` |

## `sweep_test.go`

| Go test | Status | Rust test |
| --- | --- | --- |
| TestSweepDryRunWritesNothing | covered | `tests/sweep.rs` `a_dry_run_writes_nothing` |
| TestSweepWithdrawsOpenCasesAndLeavesTheRest | changed, because ST-14: sweep's withdraws are the human's, with the human's actor | `tests/sweep.rs` `sweep_withdraws_open_cases_as_the_human_and_leaves_the_rest` |
| TestSweepFilters | changed, because ST-14: `--older-than` reads the case's last event, as list does | `tests/sweep.rs` `sweep_filters_by_label_worker_kind_and_the_age_of_the_last_event` |
| TestSweepLeavesACaseChangedSinceItWasListed | covered, as a unit test: the change has to land between sweep's read and its write, which a test outside the process cannot time | `sweep::tests` `a_case_changed_since_it_was_read_is_left` |
| TestSweepByKind | covered | `tests/sweep.rs` `sweep_filters_by_label_worker_kind_and_the_age_of_the_last_event` |
| TestSweepWithdrawsThroughTheStore | dropped, because it swaps in a Go store whose withdraw fails. Sweep treats every withdraw that fails alike, going on with the rest and naming it at the end, which `sweep::tests` `a_case_changed_since_it_was_read_is_left` checks with a stale one |  |

## `wait_test.go`

| Go test | Status | Rust test |
| --- | --- | --- |
| TestWaitReturnsAnAnswerThatLands | covered | `tests/wait.rs` `wait_returns_an_answer_that_lands` |
| TestWaitTimesOutWithExitTwo | covered, and it gives up at the timeout, not at its next read of the store | `tests/wait.rs` `wait_times_out_with_exit_2_and_nothing_on_stdout` |
| TestWaitIgnoresEventsFromBeforeItStarted | covered | `tests/wait.rs` `wait_ignores_events_from_before_it_started` |
| TestWaitSinceCountsEarlierEvents | covered | `tests/wait.rs` `wait_since_counts_events_already_in_the_store` |
| TestWaitParkAndResume | changed, because HU-05: the agent does not resume a case, so an agent note and withdraw stand in for Go's `resume --agent` as the agent's own events that do not wake it | `tests/wait.rs` `wait_wakes_on_a_park_and_a_resume_and_not_on_the_agent_s_own_events` |
| TestWaitOnSpecificCases | covered, and an id may have a title slug after it (ST-10) while a part of an id is refused (HU-07) | `tests/wait.rs` `wait_on_cases_by_their_whole_id` |
| TestWaitByLabelAndWorker | covered | `tests/wait.rs` `wait_by_label_and_worker` |
| TestWaitByIDAndLabel | covered | `tests/wait.rs` `wait_by_id_and_label_waits_on_cases_that_pass_both` |
| TestWaitForAStoreThatDoesNotExistYet | covered, and wait does not make the store (ST-08) | `tests/wait.rs` `wait_waits_for_a_store_that_is_not_there_yet` |
| TestWaitReportsAnAnswerWithoutTimestamp | covered, and with no time among the events it prints and no `--since` there is no `next_since` (AG-04). Cases whose events have one time print in the order they were opened, not by id (ST-10) | `tests/wait.rs` `answers_with_no_time_are_new_when_they_land_and_print_in_open_order` |
| TestWaitWarnsOnceAboutAMalformedAnswer | covered, and for a case that does not fold | `tests/wait.rs` `wait_warns_once_about_a_case_or_row_that_does_not_fold` |
| TestWaitForHuman | covered | `tests/wait.rs` `wait_for_human_wakes_when_the_agent_hands_a_case_over` |
| TestWaitForHumanAmendAndResume | changed, because HU-05: the agent does not resume a case, so the check that an agent resume hands a parked case to the human goes. The rest is covered | `tests/wait.rs` `wait_for_human_passes_over_amends_and_the_human_s_own_events` |
| TestWaitSinceCaseID | covered | `tests/wait.rs` `wait_since_a_case_id_counts_from_its_open` |
| TestWaitPrintsTheNextSince | covered | `tests/wait.rs` `wait_prints_the_next_since` |
| TestWaitMarksWhichCasesAreFresh | covered | `tests/wait.rs` `wait_marks_which_cases_are_fresh` |
| TestWaitPickupPicksUpTheAnswer | covered | `tests/wait.rs` `wait_pickup_picks_up_the_answer` |
| TestWaitPickupLeavesParkedAndResumedCases | changed, because AG-04 and AG-05: a case the human withdrew is printed as it is too, not picked up | `tests/wait.rs` `wait_pickup_leaves_parked_resumed_and_withdrawn_cases` |
| TestWaitPickupSkipsACaseChangedSinceItWasRead | covered. The test holds the store's lock while it writes the change, in place of Go's store that changes the case first | `tests/wait.rs` `wait_pickup_leaves_a_case_that_changed_since_it_was_read` |
| TestWaitPickupByTwoWaitsPicksUpOnce | covered. The test holds the store's lock until both waits have read the answer, in place of Go's barrier | `tests/wait.rs` `two_pickup_waits_on_one_case_pick_it_up_once` |
| TestWaitPickupByLabel | covered | `tests/wait.rs` `wait_pickup_by_label` |
| TestWaitPickupFlags | covered | `tests/wait.rs` `wait_pickup_refuses_what_could_pick_up_other_agents_cases` |

## `withdraw_test.go`

| Go test | Status | Rust test |
| --- | --- | --- |
| TestWithdraw | covered, and an answered case is not withdrawn | `tests/agent.rs` `withdraw_takes_back_an_open_case` |
| TestWithdrawAtRevision | changed, because CL-01 and AG-12: a stale write exits 4 | `tests/agent.rs` `withdraw_at_a_revision_the_case_has_left_is_refused_as_stale` |
| TestWithdrawReason | changed, because ST-12: the record also carries the writer's actor | `tests/agent.rs` `withdraw_records_its_reason` |

## New here

Tests for what the Go CLI did not have, or did not test:

- A parse error exits 1, not clap's 2, which is `wait`'s timeout (CL-01): `tests/global.rs` `bad_input_exits_1_not_clap_s_2`.
- `park` is a command of its own (HU-03): `tests/global.rs` `park_is_a_command_of_its_own`, `tests/park.rs`.
- Every write's `--json` prints the id, state and new revision (CL-02): the `…_refused_as_stale` tests in `tests/amend.rs`, `tests/answer.rs`, `tests/park.rs` and `tests/agent.rs`.
- An answer carries the amend it saw (ST-04): `tests/answer.rs` `an_answer_carries_the_amend_it_saw`.
- An answer on a case that is not open exits 3: `tests/answer.rs` `answer_on_a_case_that_is_not_open_exits_3`.
- Sweep on a store that is not there makes nothing: `tests/sweep.rs` `sweep_on_a_store_that_is_not_there_does_nothing`.
- `open --json` prints the id, state and revision (CL-02): `tests/open.rs` `open_json_prints_the_id_state_and_revision`.
- Open writes one event, seq 1, by the agent, with `opened_at` in UTC, and prints the id (AG-01): `tests/open.rs` `open_prints_the_new_case_id_and_writes_one_open_event`.
- One agent actor per worker name, named in `show --json` (AG-01, AG-13): `tests/open.rs` `open_records_the_worker_as_the_actor`, `tests/show.rs` `show_names_each_actor_from_the_actors_table`.
- Unknown fields are kept in `show --json` (AG-13, ST-05): `tests/show.rs` `show_json_keeps_each_record_as_stored`.
- `show --json` and `list --json` give the case's amend seq (AG-13, ST-04): `tests/show.rs` `show_json_gives_the_amend_seq`, `tests/list.rs` `list_filters_by_state_and_prints_json`.
- An unnamed actor is shown as its id and kind, and one whose row is deleted as its id alone (AG-13): `tests/show.rs` `an_actor_without_a_name_or_a_row_is_shown_by_what_is_known`.
- A broken pipe on stdout, as `cases list | head -1` makes, prints no error: `error::tests` `a_broken_pipe_fails_quietly`.
- The config file's keys and errors, which Go tested in `internal/config`: `config::tests`.
- Durations as flags take them: `duration::tests`. Text from a flag, a file or stdin (CL-03): `text::tests`. The table, ages and times: `output::tests`. The instance file's name: `instance::tests`.
- The skill is the one written for this CLI (SK-02), and every `cases` command and subcommand it names is in the command tree: `commands::skill::tests` `every_command_the_skill_names_exists`.
- The service manager, ported from Go's `internal/service` tests: `service::tests`. The loopback check on `--listen`: `listen::tests`. The browser opener, `launchctl` and `systemctl` are found on `$PATH` only, never in `/usr/bin` or `/bin` when it is unset: `program::tests`, `tests/inbox.rs` `inbox_without_a_browser_opener_fails`, `tests/service.rs` `without_the_service_manager_on_path_nothing_is_touched`.
- `config show` and `config path` run when no store or config file can be found: `tests/config.rs` `config_show_gives_the_store_under_the_data_home_by_default`.
- An instance file that cannot be read is an error naming it on `status`: `tests/status.rs` `an_instance_file_that_cannot_be_read_is_an_error_naming_it`.
- A human withdraw, as sweep writes, wakes `wait`, which prints the case as withdrawn once and never again, and an agent withdraw does not (AG-04): `tests/wait.rs` `a_human_withdraw_wakes_wait_once`.
- AG-03's recipe, `wait --id ID --since ID --pickup`, picks up an answer that landed before the wait started, and no later wait prints the case: `tests/wait.rs` `the_recipe_catches_an_answer_that_lands_before_the_wait_starts`.
- A `wait --pickup` that times out writes nothing: `tests/wait.rs` `a_pickup_wait_that_times_out_writes_nothing`.
- `wait --pickup` picks up each case just before it prints it, so a wait whose stdout has gone leaves the cases after the one it was on answered for the next wait, and names on stderr the one it picked up but could not print (AG-03): `tests/wait.rs` `a_pickup_wait_that_cannot_print_leaves_the_cases_after_answered`.
- A pickup that makes its actor, for a case opened by hand, is named in `wait`'s line as `show --json` names it (AG-13): `tests/wait.rs` `a_pickup_that_makes_its_actor_names_it`.
- Each case `wait` prints is one line, even when a record stored by hand or by another tool holds line breaks, or a string holds NEL or a line or paragraph separator (AG-04): `tests/wait.rs` `each_case_is_one_line_whatever_its_records_hold`.
- A store thrown away and made again while `wait` runs gives its cases the old store's keys, from 1 (ST-10). Every event in it is new to the wait (ST-08), and `--id` keeps naming the case the wait first found with that id, never another agent's case with the same id (AG-05); a wait by id on a store not there yet waits for the store: `tests/wait.rs` `every_event_of_a_store_made_again_while_wait_runs_is_new`, `an_id_names_the_case_it_named_when_the_wait_started`, `wait_waits_for_a_store_that_is_not_there_yet`.
- An empty `--since`, and a `--by` without `--pickup` even when empty, are refused (exit 1). Go took an empty one as not given, so `--since "$NEXT"` with the variable unset waited without `--since`: `tests/wait.rs` `wait_since_counts_events_already_in_the_store`, `wait_pickup_refuses_what_could_pick_up_other_agents_cases`.

# Parity with the Go web inbox

Every test of the Go web inbox, `internal/web` and `internal/notify` in `ryanlewis/cases` at 508548b, and what covers it here, marked as above, and one more: **by hand**, the page draws it, which no test here runs, and it was checked by hand in a browser; any test named checks only the part the API or the page's files hold. The inbox is now an SPA over a JSON API (spec/api.md, and IN-01 to IN-12 restated for it), so a test of HTML the Go server drew is covered here by a test of the API where the behaviour lives in the API. Where it lives in the page, which the SPA draws in the browser, the row says so: no test here runs a browser. Those were checked by hand in headless Chromium against a running serve, by scripts kept out of the repository: every view and kind of form, the keys, the options, notifications, a note and an amend landing while a form is open, two tabs sharing one stream, and a page reloading for a new build (IN-12). `tests/api.rs` runs the built binary's `serve` and talks HTTP to it; `commands::serve::tests` runs the server in the test's process, for timeouts too long to wait out.

Tally: 89 Go tests, 37 covered, 6 changed, 43 by hand, 3 dropped, 0 pending.

### `internal/web/events_test.go`

| Go test | Status | Rust test |
| --- | --- | --- |
| TestEventStreamSendsEachChange | covered | `tests/api.rs` `the_stream_sends_each_change_and_catches_up_after_a_reconnect` |
| TestEventStreamHoldsMessagesApart | covered | `tests/api.rs` `the_stream_holds_its_messages_a_second_apart` |
| TestEventVersionOutlivesTheServer | covered | `tests/api.rs` `the_stream_sends_each_change_and_catches_up_after_a_reconnect`; `commands::serve::watch::tests` `the_version_moves_with_each_event_and_nothing_else` |
| TestEventStreamEndsWhenThePageGoes | covered | `tests/api.rs` `a_stream_ends_when_its_page_goes_away` |
| TestEventStreamEndsWhenThePageStopsReading | covered: every write to a connection must go out within 10 s, or the connection is closed; tested with a writer that never takes a byte | `commands::serve::http::tests` `a_write_that_cannot_go_out_fails_at_the_timeout`, `a_write_that_goes_out_clears_the_deadline` |
| TestEventStreamOutlivesTheReadTimeout | covered | `commands::serve::tests` `a_stream_outlives_the_read_timeout` |
| TestServeClosesStreamsWhenTheListenerFails | dropped: the Go test closes the listener under the server, which tokio's listener cannot be; an accept that fails is tried again after a pause, as `axum::serve` does |  |
| TestServeEndsOpenStreams | covered: the streams end cleanly when serve stops, and one opened as it stops ends at once with the head that makes a browser try again | `tests/serve.rs` `serve_pushes_changes_and_stops_with_a_stream_open`; `commands::serve::tests` `a_stream_opened_while_serve_stops_ends_at_once` |
| TestPagesFollowTheStore | by hand, because IN-06: the page holds one event stream per browser, from its visible tab, and passes each version to the other tabs, which read the store at once; the stream is not logged. Drawn by the page, which was checked in a browser by hand, not by a test here. | `commands::serve::page::tests` `the_page_follows_the_store_and_keeps_markdown_safe`; `tests/serve.rs` `serve_logs_writes_and_failures_but_not_a_page_s_own_reads`; `tests/api.rs` `the_stream_sends_each_change_and_catches_up_after_a_reconnect` |

### `internal/web/favicon_test.go`

| Go test | Status | Rust test |
| --- | --- | --- |
| TestFaviconDrawsTheCount | covered, and a drawn icon is not logged where a refused one is | `commands::serve::favicon::tests` `the_icon_draws_the_count`; `tests/api.rs` `the_tab_icon_is_drawn_for_its_query`; `tests/serve.rs` `serve_logs_writes_and_failures_but_not_a_page_s_own_reads` |
| TestFaviconFollowsTheTitleCount | by hand, because IN-01: the page points the icon at the count of the open cases it read, red when one is blocking; a parked case is not counted. Drawn by the page, which was checked in a browser by hand, not by a test here. | `tests/api.rs` `the_tab_icon_is_drawn_for_its_query`, `the_list_is_list_json_in_inbox_order` |

### `internal/web/keys_test.go`

| Go test | Status | Rust test |
| --- | --- | --- |
| TestKeyboardShortcuts | by hand, because IN-09: the keys are the SPA's; a test checks that they find the form, send button, rows and cards the views draw. Drawn by the page, which was checked in a browser by hand, not by a test here. | `commands::serve::page::tests` `the_keys_find_what_the_page_draws` |

### `internal/web/notifications_test.go`

| Go test | Status | Rust test |
| --- | --- | --- |
| TestNotificationsFeed | covered, and the checks are not logged | `tests/api.rs` `the_notification_feed_numbers_what_lands_on_the_human`; `tests/serve.rs` `serve_logs_writes_and_failures_but_not_a_page_s_own_reads` |
| TestNotificationsKeepTheLastFifty | covered | `commands::serve::notify::tests` `the_feed_keeps_the_last_items_and_counts_on` |
| TestNotificationsBootIDChangesWithTheServer | covered | `tests/api.rs` `the_notification_feed_numbers_what_lands_on_the_human` |
| TestNotificationsRefusals | covered: a bad `after` is 400, a foreign Host 403, and a POST 405 | `tests/api.rs` `the_notification_feed_numbers_what_lands_on_the_human`, `the_host_must_be_this_server`, `every_refusal_has_its_status_and_writes_nothing` |
| TestNotificationsWhenTheStoreArrivesLater | covered | `commands::serve::watch::tests` `a_missing_store_reads_as_empty_and_its_first_case_notifies` |
| TestNotificationsWhenTheStoreIsPutBack | covered | `commands::serve::watch::tests` `a_store_put_back_does_not_notify_again` |

### `internal/web/views_test.go`

| Go test | Status | Rust test |
| --- | --- | --- |
| TestInboxOrderAndContent | by hand, because IN-01: the order and each case's fields come from the API, which the page lists as it gives them; a refresh of / keeps the case it shows while that case waits, as Go's did. Drawn by the page, which was checked in a browser by hand, not by a test here. | `tests/api.rs` `the_list_is_list_json_in_inbox_order` |
| TestEachKindRendersAndAnswers | by hand, because IN-03: each kind's form is the SPA's, and a send is one request that writes one human event. Drawn by the page, which was checked in a browser by hand, not by a test here. | `tests/api.rs` `an_answer_writes_one_event_as_serves_human`, `drop_answers_every_kind`, `park_and_resume_are_writes_of_their_own`, `an_answer_without_its_response_is_refused_for_every_kind` |
| TestDropAnswersEveryKind | by hand, because IN-03: every kind's form has drop. Drawn by the page, which was checked in a browser by hand, not by a test here. | `tests/api.rs` `drop_answers_every_kind` |
| TestAmendedCase | by hand, because IN-02 and IN-06: the case is marked as amended, an amend that changes the question leaves the form at its revision, and the send is refused and the case shown again. Drawn by the page, which was checked in a browser by hand, not by a test here. | `tests/api.rs` `an_answer_that_missed_an_amend_is_stale`, `a_refusal_carries_the_case_as_it_is_now` |
| TestThreadShowsWhatAnAmendReplaced | by hand, because IN-02: the thread is drawn from each event's record, with a replaced body or context in full. Drawn by the page, which was checked in a browser by hand, not by a test here. | `tests/show.rs` `show_prints_what_an_amend_replaced` |
| TestThreadDetailsFollowTheText | by hand, because IN-06: a redraw keeps the page's elements, so a details opened stays open; there are no ids to keep in step with the text. Not checked in a browser. |  |
| TestBodyRendersGFMTable | by hand, because IN-02: marked draws the body, with GFM tables. Drawn by the page, which was checked in a browser by hand, not by a test here. | `commands::serve::page::tests` `the_page_follows_the_store_and_keeps_markdown_safe` |
| TestBodyAlignsTableColumnsByClass | by hand, because IN-02: marked draws the body, and a column's alignment is a class, as in Go. Drawn by the page, which was checked in a browser by hand, not by a test here. | `commands::serve::page::tests` `the_stylesheet_keeps_its_rules` |
| TestStuckParkAndResume | by hand, because IN-03 and HU-03: park is a button of a stuck case's form, and resume a parked case's form, each its own request. Drawn by the page, which was checked in a browser by hand, not by a test here. | `tests/api.rs` `park_and_resume_are_writes_of_their_own` |
| TestPostsGoToTheNextCase | by hand, because IN-03: a send is a request, not a form post and redirect; the page goes to the next case in the inbox as it stood before the send, or to / after the last. Drawn by the page, which was checked in a browser by hand, not by a test here. |  |
| TestRedirectConfirmsWhatWasRecorded | by hand, because IN-03: the page says once which event it recorded on which case, and the line goes by itself; being in memory, a reload does not show it again. Drawn by the page, which was checked in a browser by hand, not by a test here. | `commands::serve::page::tests` `the_stylesheet_keeps_its_rules` |
| TestInvalidAnswerWritesNothing | by hand, because IN-04: the API's 422 writes nothing, and the page shows the reason and keeps what was typed. Drawn by the page, which was checked in a browser by hand, not by a test here. | `tests/api.rs` `every_refusal_has_its_status_and_writes_nothing` |
| TestAnswerOnAClosedCaseOrUnknownCase | covered: an answer on a withdrawn case is 422, and on an unknown case 404, and neither writes | `tests/api.rs` `every_refusal_has_its_status_and_writes_nothing`, `a_store_not_made_yet_lists_no_cases` |
| TestStaleTabCannotAnswerAReopenedCase | covered: a write at a revision the case has left is 409, and writes nothing | `tests/api.rs` `a_refusal_carries_the_case_as_it_is_now`, `every_refusal_has_its_status_and_writes_nothing` |
| TestStaleParkAndResumeAreRefused | covered | `tests/api.rs` `park_and_resume_are_writes_of_their_own` |
| TestStaleRefusalShowsTheCaseAsItIsNow | by hand, because IN-05: the 409 carries the case as it is now, which the page shows, keeping what was typed, at its new revision. Drawn by the page, which was checked in a browser by hand, not by a test here. | `tests/api.rs` `a_refusal_carries_the_case_as_it_is_now` |
| TestCaseViewRefresh | by hand, because IN-06: the page reads the case again on each stream message, and redraws it in place, or in its new state. Drawn by the page, which was checked in a browser by hand, not by a test here. | `tests/api.rs` `the_stream_sends_each_change_and_catches_up_after_a_reconnect` |
| TestCaseViewKeepsTheFormAsLeft | by hand, because IN-06: the form's fields are the page's state, which a redraw keeps; the form takes the new revision after a note, and not after an amend that changes the question, nor when the case was answered, parked or resumed between two reads, which Go's form took (IN-05). Drawn by the page, which was checked in a browser by hand, not by a test here. |  |
| TestFormIDsMatchTheirFields | dropped: the ids were what htmx kept fields by across a refresh; Preact keeps them by redrawing in place |  |
| TestRefusalLineOutlivesARefresh | by hand, because IN-06: the refusal line is the case view's state, which a redraw keeps, and a refusal on / keeps its case shown, on its own page. Drawn by the page, which was checked in a browser by hand, not by a test here. |  |
| TestOldThreadPoll | dropped, because IN-12: the Go shim for pages from before pages followed the store goes (fresh start); an SPA tab from another build reloads instead |  |
| TestRefusedPageFollowsAChangeMadeWhileItWasSent | by hand, because IN-05 and IN-06: a refused send shows the case the refusal carries, and the page follows the stream from there. Drawn by the page, which was checked in a browser by hand, not by a test here. | `tests/api.rs` `a_refusal_carries_the_case_as_it_is_now` |
| TestDoneView | by hand, because IN-07: the page reads the four done states from the API and lists them newest first, with a closed case's outcome. Drawn by the page, which was checked in a browser by hand, not by a test here. | `tests/api.rs` `the_list_is_list_json_in_inbox_order` |
| TestDoneFilters | by hand, because IN-07: the filters are the page's, and closed today is from midnight in the browser's zone. Drawn by the page, which was checked in a browser by hand, not by a test here. |  |
| TestDoneFiltersEmpty | by hand, because IN-07. Drawn by the page, which was checked in a browser by hand, not by a test here. |  |
| TestDoneCaseSitsBesideTheDoneList | by hand, because IN-02: a done case is drawn beside the done list, under in flight or all. Drawn by the page, which was checked in a browser by hand, not by a test here. |  |
| TestEmptyHomeReloadsWhenACaseArrives | by hand, because IN-01 and IN-06: at inbox zero the page reads the store on each stream message, and shows the first case that arrives. Drawn by the page, which was checked in a browser by hand, not by a test here. |  |
| TestThreadShowsWithdrawReason | by hand, because IN-02: the thread is drawn from each event's record. Drawn by the page, which was checked in a browser by hand, not by a test here. | `tests/show.rs` `show_prints_the_case_and_its_thread` |
| TestOnlyWebLinksAreAnchors | by hand, because IN-02: every link, in markdown and a case's or row's, follows one rule: an absolute address is a link unless its scheme is javascript, vbscript, data, file, blob, filesystem or about, read as the browser reads it; a relative one is text (case 2026-09-25T20-55-32Z-markdown-links-only-http-s-mailto-clickable). Drawn by the page, which was checked in a browser by hand, not by a test here. | `commands::serve::page::tests` `the_link_rule_links_absolute_addresses_but_blocked_schemes`, `the_page_follows_the_store_and_keeps_markdown_safe` |
| TestCasePageShowsTheBrief | by hand, because IN-02. Drawn by the page, which was checked in a browser by hand, not by a test here. |  |
| TestTimestampsAreLocalWithAnAge | by hand, because IN-02: times are shown in the browser's zone, with an age redrawn once a minute. Drawn by the page, which was checked in a browser by hand, not by a test here. |  |
| TestApprovalRowNote | by hand, because IN-02. Drawn by the page, which was checked in a browser by hand, not by a test here. |  |
| TestExternalLinksOpenInANewTab | by hand, because IN-02 and IN-10: links that leave the inbox open in a new tab with noopener noreferrer, unless the links option says this tab. Drawn by the page, which was checked in a browser by hand, not by a test here. | `commands::serve::page::tests` `the_page_follows_the_store_and_keeps_markdown_safe`, `the_theme_applies_before_the_stylesheet` |
| TestEmptyAnswersAreRefused | covered for every kind: the API refuses an answer with no response, 422, and writes nothing | `tests/api.rs` `an_answer_without_its_response_is_refused_for_every_kind` |
| TestChoicesAreRequiredInTheBrowser | by hand, because IN-03: the page's choices are required, and it does not send a form missing its response; the API refuses one that arrives anyway. Drawn by the page, which was checked in a browser by hand, not by a test here. | `tests/api.rs` `an_answer_without_its_response_is_refused_for_every_kind` |
| TestTallyHidesZeros | by hand, because IN-01: the tally is the page's, from the open and parked cases it read. Drawn by the page, which was checked in a browser by hand, not by a test here. |  |
| TestThreadShowsAnUnknownEventAsVersionSkew | by hand, because IN-02 and ST-05: a row the fold skipped is in the case's problems, which the page shows under the thread. Drawn by the page, which was checked in a browser by hand, not by a test here. | `tests/show.rs` `show_reports_an_unknown_event_as_version_skew` |
| TestAnswerWritesThroughTheStore | covered: one `human answer` event, by serve's human | `tests/api.rs` `an_answer_writes_one_event_as_serves_human` |
| TestAnswerIsRecordedWhenTheBrowserGoesAway | covered, with the write held on the store's lock while the page goes, and with the write waiting its turn behind another, which is logged too | `tests/api.rs` `a_write_is_made_even_when_the_page_goes_away`, `a_write_waiting_its_turn_is_made_and_logged_when_its_page_goes_away` |
| TestInboxZeroCountsWhatWasGotThrough | by hand, because IN-01 and IN-07: the figures are worked out in the page from every case, with today and this week from midnight and Monday in the browser's zone. Drawn by the page, which was checked in a browser by hand, not by a test here. |  |
| TestInboxZeroHidesZeroLines | by hand, because IN-01. Drawn by the page, which was checked in a browser by hand, not by a test here. | `tests/api.rs` `a_store_not_made_yet_lists_no_cases` |
| TestThreadShowsActorAndFor | by hand, because IN-02 and AG-13: each actor is named from `show --json`: its name, its id when it has none, and deleted user when its row is gone. Drawn by the page, which was checked in a browser by hand, not by a test here. | `tests/show.rs` `show_names_each_actor_from_the_actors_table`, `an_actor_without_a_name_or_a_row_is_shown_by_what_is_known` |
| TestTitleCountsCasesWaitingOnTheHuman | by hand, because IN-01: the title is the page's, from the open cases it read. Drawn by the page, which was checked in a browser by hand, not by a test here. |  |

### `internal/web/web_test.go`

| Go test | Status | Rust test |
| --- | --- | --- |
| TestCheckLoopback | covered | `listen::tests`; `tests/serve.rs` `serve_refuses_an_address_that_is_not_loopback` |
| TestHostSpellings | covered | `commands::serve::app::tests` `a_host_is_the_listen_address_or_localhost`; `tests/api.rs` `the_host_must_be_this_server` |
| TestSecurityHeaders | changed, because IN-11: the page has one inline script, its import map, which the policy names by hash; every response has the headers | `commands::serve::page::tests` `the_policy_names_the_import_map_by_its_hash`; `tests/page.rs` `the_page_s_policy_names_its_one_inline_script`; `tests/api.rs` `every_response_carries_the_security_headers_and_no_cors` |
| TestHostMustBeTheServer | covered | `tests/api.rs` `the_host_must_be_this_server` |
| TestCrossSitePostsAreRejected | covered, with the CSRF token as well (IN-11) | `tests/api.rs` `every_refusal_has_its_status_and_writes_nothing` |
| TestStaticHTMXIsTheRecordedRelease | changed, because IN-11: Preact, its hooks, htm and marked replace htmx, and each is checked against the release its README records | `commands::serve::page::tests` `each_vendored_file_is_the_recorded_release`; `tests/page.rs` `the_spa_s_files_are_served_with_a_tag` |
| TestStyleHasViewTransitions | covered for the rules. The page moves between pages without loading one, so it asks for the cross-fade itself. | `commands::serve::page::tests` `the_stylesheet_keeps_its_rules` |
| TestStyleCentresInboxZero | covered | `commands::serve::page::tests` `the_stylesheet_keeps_its_rules` |
| TestRecordedLineGoesByItself | by hand, because IN-03: the rules are kept, and the page takes the line away when its animation ends, from memory, so no query is left to take off the address. Drawn by the page, which was checked in a browser by hand, not by a test here. | `commands::serve::page::tests` `the_stylesheet_keeps_its_rules` |
| TestStyleSpansTheSplit | covered | `commands::serve::page::tests` `the_stylesheet_keeps_its_rules` |
| TestStyleKeepsBoldReadableOnHover | covered | `commands::serve::page::tests` `the_stylesheet_keeps_its_rules` |
| TestStyleScalesWithTextSize | covered | `commands::serve::page::tests` `the_stylesheet_scales_with_the_text_size` |
| TestMarkdownCannotInjectMarkup | by hand, because IN-02: marked draws markdown in the browser and drops raw HTML, a `javascript:` link is text, and the policy blocks script besides. Drawn by the page, which was checked in a browser by hand, not by a test here. | `commands::serve::page::tests` `the_page_follows_the_store_and_keeps_markdown_safe`, `the_policy_names_the_import_map_by_its_hash` |
| TestLabelsAreShownAsText | by hand, because IN-02: Preact draws every field as text. Drawn by the page, which was checked in a browser by hand, not by a test here. |  |
| TestRequestsAreLogged | changed, because SV-02: a page's own reads are API reads, not htmx fragments, and are left out when they succeed. A 500 logs its cause on the line before, as Go's `fail` did | `tests/serve.rs` `serve_logs_writes_and_failures_but_not_a_page_s_own_reads`, `a_request_that_fails_logs_why` |
| TestMissingStoreShowsAnEmptyInbox | by hand, because IN-01: the API lists no cases, and the page shows inbox zero. Drawn by the page, which was checked in a browser by hand, not by a test here. | `tests/api.rs` `a_store_not_made_yet_lists_no_cases` |
| TestOptionsOverlayAndPrefsScript | by hand, because IN-10: the options dialog is the page's, and theme.js keeps the choices and applies them before the page paints. Drawn by the page, which was checked in a browser by hand, not by a test here. | `commands::serve::page::tests` `the_theme_applies_before_the_stylesheet` |
| TestFormsGrowWithTheirText | changed, because IN-03: every free-text field is a textarea that grows with its text, by the stylesheet's field-sizing, or by its scroll height where the browser lacks it; the browser the page was checked in has field-sizing, so the scroll-height path ran nowhere. | `commands::serve::page::tests` `the_stylesheet_keeps_its_rules` |
| TestServeDropsAStalledPost | covered, and a request whose head never ends is dropped too | `commands::serve::tests` `a_stalled_request_is_dropped_at_the_read_timeout` |
| TestServeClosesWhatShutdownLeaves | changed: serve logs how many connections it closed and exits 0, where Go returned the shutdown's timeout as an error | `commands::serve::tests` `a_stop_closes_what_the_grace_period_leaves` |

### `internal/notify/notify_test.go`

| Go test | Status | Rust test |
| --- | --- | --- |
| TestFirstPollOnlyRecordsTheStore | covered | `commands::serve::notify::tests` `the_first_read_only_records_the_store` |
| TestEmptyFirstPollStillBaselines | covered | `commands::serve::notify::tests` `an_empty_first_read_still_counts` |
| TestWhatLandsOnTheHuman | changed, because HU-05: the agent never resumes a case, so an agent resume no longer lands one; the rest holds | `commands::serve::notify::tests` `what_lands_on_the_human` |
| TestACaseOpenedAndFollowedUpBetweenPollsFiresOnce | covered | `commands::serve::notify::tests` `a_case_opened_and_followed_up_between_reads_lands_once_by_its_open` |
| TestACaseAlreadyAnsweredWhenSeenDoesNotFire | covered | `commands::serve::notify::tests` `a_case_answered_before_it_was_seen_does_not_land` |
| TestSeenForgetsCasesThatLeaveTheStore | covered | `commands::serve::notify::tests` `a_case_that_leaves_the_store_is_forgotten` |
| TestItemPayload | covered | `commands::serve::notify::tests` `an_item_says_what_landed_and_where_it_is` |
| TestFeedKeepsTheLastItemsAndCountsOn | covered | `commands::serve::notify::tests` `the_feed_keeps_the_last_items_and_counts_on` |

## New here

- The API's writes and every status code spec/api.md pins, in the order it gives: `tests/api.rs` `every_refusal_has_its_status_and_writes_nothing`, `the_checks_come_in_the_contract_s_order`.
- A write carries the CSRF token `GET /v1` gives, new each time serve starts (IN-11): `tests/api.rs` `v1_gives_the_build_and_a_token_new_each_run`, `every_refusal_has_its_status_and_writes_nothing`.
- An answer that missed an amend is stale even at the case's revision (ST-04): `tests/api.rs` `an_answer_that_missed_an_amend_is_stale`.
- The case list is `list --json`'s, in inbox order, by state: `tests/api.rs` `the_list_is_list_json_in_inbox_order`. A case is `show --json`'s: `tests/api.rs` `a_case_is_what_show_json_gives`.
- A write that is made is a 200 with the case even when the actors cannot be read after it: `tests/api.rs` `a_write_made_is_a_200_even_when_the_actors_cannot_be_read`.
- A store thrown away and made again at the path starts its keys again at 1, and is still a new store: its cases notify, and its version is not the old store's. Go's case ids held the open time, so it never had to tell them apart: `commands::serve::notify::tests` `a_case_of_a_store_made_again_lands_though_its_key_was_seen`, `commands::serve::watch::tests` `another_store_with_the_same_keys_has_another_version`.
- Every page path is the one page, and the SPA's files are served with their type, an `ETag` and `no-cache`, and a 304 when the page has them (IN-12): `tests/page.rs` `every_page_path_is_the_page`, `the_spa_s_files_are_served_with_a_tag`. `GET /v1` gives the build, a hash of the page's files, which a page compares with the build the page names, so a page from the browser's cache knows it too: `tests/page.rs` `v1_gives_the_build_of_the_page`; `commands::serve::page::tests` `the_page_names_its_build`.
- Every module the SPA imports is served, and the import map resolves each bare name: `commands::serve::page::tests` `every_module_imported_is_served`.
- serve closes the store when it stops, so SQLite folds its write-ahead log back into the file, and a copy of the file has every write: `tests/serve.rs` `serve_closes_the_store_when_it_stops`; `commands::serve::watch::tests` `a_watch_that_drops_closes_the_store`.
- The instance file serve writes, ported from Go's `internal/instance` tests TestRemoveLeavesAnotherServesFile, TestWriteReplacesAndLeavesNoTemporaryFiles and TestWriteFailsWhenStateDirectoryIsAFile: `instance::tests` `a_serve_removes_only_its_own_file`, `a_write_replaces_the_file_and_leaves_nothing_else`, `a_write_fails_when_the_state_directory_is_a_file`.
