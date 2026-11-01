# Parity with the Go store

Every test of the Go store, `internal/store` in `ryanlewis/cases` at 508548b, and what covers it here. Each one is:

- **covered**: a test here checks the same behaviour;
- **changed**: v1 changes the behaviour, for the story or decision named, and a test here checks the new behaviour;
- **dropped**: the test is of Go-only code, for the reason given.

The Go tests of the core, `vectors_test.go` and `fold_test.go`, are left out: `cases-core` runs every vector itself (`crates/core/tests/vectors.rs`). The part of `vectors_test.go` that ran each append through the Go store, `checkStore`, is here as the store vector run, `tests/vectors.rs`. `describe_test.go` tests the text of a case's thread, which the CLI owns.

Tests here are named by file and function: `tests/write.rs` is an integration test, and `db::tests` a unit test in `src/db.rs`. `cases_id::tests` are in `crates/id`.

Tally: 53 Go tests, 32 covered, 19 changed, 2 dropped, none left unmatched.

## `write_test.go`

| Go test | Status | Rust test |
| --- | --- | --- |
| TestCreateWritesOpenEvent | changed, because ST-10: the id is the key's Sqids encoding, so the same title in the same second is the next key with its own id, not `-2` | `tests/write.rs` `create_writes_the_open_event` |
| TestCreateWritesLabels | covered | `tests/write.rs` `create_writes_labels` |
| TestOpenRecordWithoutLabelsLoads | covered; its check of the case's JSON is the CLI's output | `tests/write.rs` `a_stored_open_without_labels_folds` |
| TestCreateRefusesInvalidOpenWithoutMakingTheStore | covered, and neither is the directory made | `tests/write.rs` `a_refused_open_does_not_make_the_store` |
| TestCreateRollsBackTheCase | changed, because ST-12 and AG-01: the core refuses an open before the store touches the file, and Go's refused open, a blank actor name, is gone with names on events. A create whose commit fails keeps no case, no actor and no key | `db::tests` `a_failed_create_keeps_nothing` |
| TestAFailedCommitWritesNothing | covered | `db::tests` `a_failed_commit_writes_nothing` |
| TestSlug | changed, because ST-10: the id carries no slug, and the store makes none. A slug after an id is split off and ignored; making one from a title is the CLI's, for its display suffix | `cases_id::tests` `split_takes_off_the_slug`, `key::tests` `a_key_displays_and_parses_as_its_id` |
| TestValidIDRejectsPaths | changed, because ST-10: an id is the canonical Sqids encoding of a key, and any other text, a path included, names no case | `cases_id::tests` `only_the_id_a_key_encodes_to_names_it`, `key::tests` `what_is_no_id_names_no_case` |
| TestAppendNumbersEventsInOrder | covered | `tests/write.rs` `appends_number_events_in_order` |
| TestInvalidTransitionIsNotWritten | covered, with each refusal's category | `tests/write.rs` `a_refused_event_is_not_written` |
| TestAmend | changed, because ST-12: the amend's record also carries the writer's actor | `tests/write.rs` `an_amend_adds_to_the_case` |
| TestAppendToMissingCase | covered | `tests/write.rs` `appending_to_a_missing_case` |
| TestConcurrentAppendsTakeDistinctSequenceNumbers | covered | `tests/write.rs` `concurrent_appends_take_distinct_seqs` |
| TestTwoWritersOnOneFileSerialise | covered | `tests/write.rs` `two_writers_on_one_file_take_turns` |
| TestTwoProcessesOnOneFileSerialise | covered | `tests/write.rs` `two_processes_on_one_file_take_turns` |
| TestWriterProcess | covered | `tests/write.rs` `writer_process` |
| TestAtRevision | changed, because HU-05: resume is the human's, so the unconditional resume is too | `tests/write.rs` `a_write_at_a_revision_is_refused_once_the_case_moves_on` |
| TestAgentWritesAtRevision | changed, because ST-14: the human writes `withdraw` too, and it takes a revision as the agent's writes do | `tests/write.rs` `every_writer_writes_at_a_revision` |
| TestAtRevisionIsCheckedInsideTheWriteTransaction | covered | `tests/write.rs` `the_revision_is_checked_inside_the_write_transaction` |
| TestEventsAreStampedOnceTheWriteHoldsTheLock | covered | `tests/write.rs` `events_are_stamped_once_the_write_holds_the_lock` |
| TestWrittenRecordShape | changed, because ST-12: the record also carries the writer's actor | `tests/write.rs` `a_written_record_has_its_events_fields_and_time` |
| TestIsWholeID | changed, because ST-10: a whole id is one that decodes to a key and encodes back to itself | `cases_id::tests` `only_the_id_a_key_encodes_to_names_it`, `every_key_round_trips` |

## `store_test.go`

| Go test | Status | Rust test |
| --- | --- | --- |
| TestStoreContract | changed, because HU-05 and ST-10: resume is the human's, and an id that is not one no longer reaches the store, as it does not parse to a key. The lifecycle runs through a function generic over `Store` | `tests/store.rs` `the_store_contract` |
| TestOnlyCreateMakesTheStore | covered | `tests/store.rs` `only_create_makes_the_store` |
| TestAnEmptyFileReadsAsNoStore | covered | `tests/store.rs` `an_empty_file_reads_as_no_store` |
| TestCreateWaitsForAnotherWriterMakingTheStore | covered; and several creators racing on a missing path make one store | `tests/store.rs` `create_waits_for_another_writer_making_the_store`, `creators_racing_to_make_the_store_make_one` |
| TestCreateRefusesTheFilesOfARemovedStore | covered | `tests/store.rs` `create_refuses_the_files_of_a_removed_store` |
| TestARunningDBRefusesANewerSchema | covered | `tests/store.rs` `a_running_store_refuses_a_newer_schema` |
| TestAPoolNoCallIsUsingIsClosedWhenItsFileIsReplaced | changed, because the store keeps one connection behind its lock, not Go's pool: the connection to a replaced file is closed before the file now at the path is opened | `db::tests` `the_connection_is_opened_again_only_for_another_file`, `tests/store.rs` `a_store_replaced_again_and_again_is_followed_each_time` |
| TestAPoolInUseIsClosedOnceItsLastCallIsDone | dropped, because it tests Go's pool: with one connection behind a lock, no call holds a connection while another replaces it, so there is no connection in use to retire | |
| TestTheSchema | covered: the file's settings and constraints, the actors table's, the tables of listening's, and the connection's settings | `tests/store.rs` `the_schema`, `db::tests` `the_connection_syncs_every_commit_and_waits_for_the_lock` |
| TestOpenRefusesWhatIsNotThisStore | changed, because ST-15: a directory is still refused, without Go's pointer to the old directory store | `tests/store.rs` `a_store_refuses_what_is_not_this_store` |
| TestDBFollowsTheFileAtThePath | covered | `tests/store.rs` `the_store_follows_the_file_at_the_path` |

## `list_test.go`

| Go test | Status | Rust test |
| --- | --- | --- |
| TestLoadToleratesMalformedRows | covered, with problems compared by category, not message | `tests/list.rs` `a_case_with_damaged_rows_still_folds` |
| TestLoadPreservesUnknownFields | covered; its check of the case's JSON is the CLI's output | `tests/list.rs` `fields_this_version_does_not_know_are_kept` |
| TestLoadWithoutOpenFails | covered | `tests/list.rs` `a_case_without_an_open_does_not_fold` |
| TestListReportsBrokenCases | covered | `tests/list.rs` `a_case_that_does_not_fold_is_listed_with_the_rest` |
| TestIDs | changed, because ST-10 and AG-14: cases list by key, the order they were opened, and not by id | `tests/list.rs` `cases_list_in_the_order_they_were_opened` |
| TestPollerSeesNewEventsAndCases | covered | `tests/list.rs` `the_poller_sees_new_events_and_cases`, `poll::tests` `a_poll_folds_again_only_the_cases_that_changed` |
| TestPollerSeesACaseReplacedByAnother | covered | `tests/list.rs` `the_poller_sees_a_case_replaced_by_another` |
| TestPollerSeesACasePutBackFromTheArchive | covered | `tests/list.rs` `the_poller_sees_a_case_put_back_from_the_archive` |
| TestPollerSeesACaseArchivedAndAnotherPutBack | covered | `tests/list.rs` `the_poller_sees_a_case_archived_and_another_put_back` |
| TestPollerReadsEveryCaseAfterAFailedRefresh | covered | `poll::tests` `a_poll_after_a_failed_one_reads_every_case` |
| TestPollerStartsAgainOnANewFile | covered | `tests/list.rs` `the_poller_starts_again_on_a_new_file` |

## `archive_test.go`

| Go test | Status | Rust test |
| --- | --- | --- |
| TestArchiveMovesTheCaseOutOfSight | changed, because ST-10: no key is given twice, so a case opened after an archive gets a key of its own, not the archived id with `-2` | `tests/archive.rs` `archive_moves_the_case_out_of_sight` |
| TestArchiveRefusesAnIDTheArchiveHas | covered | `tests/archive.rs` `archive_refuses_a_key_the_archive_has` |
| TestDeleteRemovesTheCase | covered | `tests/archive.rs` `delete_removes_the_case` |
| TestArchiveAndDeleteRefuse | changed, because ST-10: an id that is not one no longer reaches the store, as it does not parse to a key | `tests/archive.rs` `archive_and_delete_refuse`, `key::tests` `what_is_no_id_names_no_case` |

## `actor_test.go`

| Go test | Status | Rust test |
| --- | --- | --- |
| TestActorAndForFold | changed, because ST-12, AG-01 and HU-06: an event's actor is an id the store gives from who writes, with the kind and name in the table of actors, and every v1 write carries one, the pickup's too | `tests/actors.rs` `each_event_carries_its_writers_actor`; the core's `fold.json` "each event keeps its actor, and the answer keeps the human's" |
| TestActorAndForValidation | changed, because ST-12: an actor is an id, with no name or kind to check, and the core checks the id's shape (`open.json` "an actor without an id is refused" and those after it). A blank `for` is `open.json` "a for of only white space is refused" | `tests/actors.rs` `actor_ids_have_the_shape_the_core_checks`, `a_records_own_actor_gives_way_to_the_writers` |
| TestActorFromALaterBuildFolds | changed, because ST-12: the core folds an actor with fields a later version adds (`fold.json` "an actor's fields other than its id are left out, and kept in the event's data"), and the store gives such a record back byte for byte | `tests/vectors.rs`, each replay |
| TestActorAndForAreIgnoredByAnEarlierBuild | dropped, because it tests Go's decoder on Go's records, and v1 starts afresh with no earlier build's records. A field this version does not know is ignored by the core: `fold.json` "fields this version does not know are kept in the event's data" | |
| TestDescribeActor | changed, because AG-13: the thread's lines are the CLI's; the store gives each actor's kind and name by id | `tests/actors.rs` `each_event_carries_its_writers_actor` |

## `storetest/storetest.go`

Its helpers, which write the store's tables with none of the store's checks, are in `tests/common/mod.rs`: `InsertEvent` is `insert_row`, with `insert_case` for a whole case, and `Archived` is `archived_rows`.

## New here

Tests for what the Go store did not have, or did not test:

- The store vector run, `tests/vectors.rs`: every vector's append through the store, and every replay stored and read back. It skips none: the store takes the author the vector gives, so an open by a human reaches the core, which takes it for a message and refuses it as `author` for any other kind.
- Actors: `tests/actors.rs` `the_first_write_by_a_name_makes_its_actor`, `a_refused_write_makes_no_actor` and `a_deleted_actor_leaves_its_events_a_bare_id`.
- The busy timeout: `db::tests` `a_write_that_waits_too_long_for_the_lock_writes_nothing`.
- The poller, built on the store's version and `rows_since`: it folds only the cases that changed, however many, `poll::tests` `a_poll_folds_only_the_changed_cases_however_many`. The poller is the core's, over any store, and its logic is tested there against a store in memory, `crates/core/tests/poller.rs`; the store's `Poller` reads with the store's own read, so a poll of a store not made yet gives `Error::Missing`.
- The store vectors (`spec/vectors/store/`), `tests/store_vectors.rs`: the version, the changes since one, the actors and the listening record, through the core's `Store`, with the vectors' clock.
- The version and `rows_since`: `read::tests` `the_version_is_the_store_the_change_the_key_the_count_and_the_row_s_time`, `the_version_moves_on_every_event_archive_and_delete_and_not_on_listening`, `a_refused_write_archive_or_delete_leaves_the_version`, `a_case_taken_out_and_another_put_back_by_hand_moves_the_version`, `rows_since_gives_what_changed_since_a_version_of_this_store`, `rows_since_gives_many_changed_cases_as_it_gives_a_few` and `rows_since_gives_every_case_for_a_past_it_cannot_trust` (another store, a copy put back from a backup and written past, a case put back by hand, a case opened since and put back by hand, a case put in by hand past the version's key).
- The store id and the logs of removals, which a store an older cases made gets when this cases first opens it, at the same schema version, so an older cases goes on using it and its deletes are logged: `schema::tests` `a_store_an_older_cases_made_has_that_cases_s_tables`, `an_older_store_is_upgraded_when_first_opened`, `an_older_store_opened_by_several_at_once_is_upgraded_once`, `an_upgraded_store_is_opened_without_a_write`, `a_store_with_part_of_the_upgrade_gets_the_rest` and `an_upgrade_that_fails_changes_nothing`.
- The change only goes up across archive and delete (ST-01): `tests/archive.rs` `the_change_only_goes_up`.
- Listeners, the waits whose `cases wait` takes messages, one row for each wait, and the workers they listen for: `listen::tests` `a_listener_is_recorded_with_its_labels_and_times`, `a_heartbeat_keeps_when_its_wait_began`, `each_wait_of_a_worker_is_its_own_row_and_lapses_on_its_own`, `a_wait_for_two_workers_is_a_row_for_each`, `a_listener_needs_a_worker_and_leaves_out_a_blank_label`, `a_row_lapses_45_seconds_after_its_last_heartbeat`, `a_worker_that_listens_again_after_its_row_lapsed_starts_afresh`, `a_doing_line_keeps_its_time_until_it_changes`, `a_doing_line_needs_a_listening_worker`, `a_doing_line_is_one_line_of_at_most_80_characters`, `a_row_that_does_not_read_is_skipped_and_deleted`, `a_listener_never_makes_the_store` and `listeners_are_listed_by_worker`, the core's `listening::tests` `a_row_lapses_three_heartbeats_after_it_was_heard`, and `poll::tests` `a_listener_moves_no_poll`. The tables are added at schema version 1 (case L5hpH): an older cases never reads them, and a store an older cases made gets them from its first listener; an older cases's one-row-per-worker `listeners` table is left alone (case sgD8j), `listen::tests` `a_store_an_older_cases_made_gets_the_tables_from_its_first_listener`, and a live row of it is read as one wait of a worker with no wait of this cases, whose doing line is set on it, `an_older_cases_s_listener_is_one_wait_until_its_worker_has_new_ones`.
- Ids: the examples of the decision, `cases_id::tests` `the_ids_of_the_examples_never_change`, and the reference implementation's ids, `ids_are_the_reference_implementations`.
