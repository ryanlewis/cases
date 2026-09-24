# What the vectors do not cover

The vectors cover the core: how a case's rows fold, the rules for each event, and how the writer numbers, stamps and checks the revision of a new event. This file lists the Go tests in `internal/store` and `cmd/cases` that check something the vectors leave out, and why. The last section lists the store tests whose rules the vectors now carry.

Reasons:

- **store**: the SQLite file and its tables, the archive, the poller, case ids.
- **concurrency**: writers running at the same time, locks, several processes.
- **text**: messages and thread lines for people, which are not part of the contract.
- **Go API**: Go types, or Go builds reading each other's records.
- **core, unreachable**: core code that no stored row or append can reach.
- **core, not yet vectorised**: core behaviour with no vector yet.
- **CLI**: commands, flags, output, exit codes, config, serve, the service and the skill.

## internal/store: 51 tests

| Test | Reason |
| --- | --- |
| `TestTransitionErrorType` | Go API: the error's type and fields. The vectors check its category, `transition` |
| `TestAnswerNeedNotSeeALabelAmend` | core, unreachable: it applies an answer numbered at or below an amend, which needs seqs out of order or given twice. `go-quirks.json` covers what can happen, through `at_revision` |
| `TestAnswerNumberIsOnlyCheckedAfterAnAmend` | core, unreachable: two events numbered 0 |
| `TestSlug` | core, not yet vectorised: the slug in a case id. Ids are outside the fold |
| `TestActorAndForAreIgnoredByAnEarlierBuild` | Go API: an earlier build's Go types reading today's records |
| `TestDescribeActor` | text: thread lines |
| `TestDescribeWithdraw` | text: thread lines |
| `TestDescribeAmend` | text: thread lines |
| `TestDescribeAmendPreviousText` | text: thread lines |
| `TestDescribeAmendSameText` | text: thread lines |
| `TestDescribe` | text: thread lines |
| `TestAmendRecordsFromAnEarlierBuild` | text: its thread lines. Its fold is in `fold.json` |
| `TestArchiveMovesTheCaseOutOfSight` | store: the archive tables |
| `TestArchiveRefusesAnIDTheArchiveHas` | store: the archive tables |
| `TestDeleteRemovesTheCase` | store: deleting a case's rows |
| `TestArchiveAndDeleteRefuse` | store: archive and delete preconditions |
| `TestListReportsBrokenCases` | store: `List` and `LoadError` across cases. A case that does not fold is in `fold.json` |
| `TestIDs` | store: listing ids |
| `TestPollerSeesNewEventsAndCases` | store: the poller and the `change` cursor |
| `TestPollerSeesACaseReplacedByAnother` | store: the poller |
| `TestPollerSeesACasePutBackFromTheArchive` | store: the poller |
| `TestPollerSeesACaseArchivedAndAnotherPutBack` | store: the poller |
| `TestPollerReadsEveryCaseAfterAFailedRefresh` | store: the poller |
| `TestPollerStartsAgainOnANewFile` | store: the poller |
| `TestStoreContract` | store: the `Store` interface by id and the errors callers branch on |
| `TestOnlyCreateMakesTheStore` | store: the file |
| `TestAnEmptyFileReadsAsNoStore` | store: the file |
| `TestCreateWaitsForAnotherWriterMakingTheStore` | concurrency: two writers making the file |
| `TestCreateRefusesTheFilesOfARemovedStore` | store: leftover `-wal` and `-shm` files |
| `TestARunningDBRefusesANewerSchema` | store: the schema version |
| `TestAPoolNoCallIsUsingIsClosedWhenItsFileIsReplaced` | store: connection pools |
| `TestAPoolInUseIsClosedOnceItsLastCallIsDone` | store: connection pools |
| `TestTheSchema` | store: the schema |
| `TestOpenRefusesWhatIsNotThisStore` | store: the file |
| `TestDBFollowsTheFileAtThePath` | store: the file |
| `TestCreateWritesOpenEvent` | store: the case id (open time and slug, `-2` on a clash) and the `cases` row. The open's number and time are in `open.json` |
| `TestCreateWritesLabels` | store: the record as stored. Labels on an open are in `open.json` |
| `TestCreateRefusesInvalidOpenWithoutMakingTheStore` | store: a refused open makes no file |
| `TestCreateRollsBackTheCase` | store: the transaction |
| `TestAFailedCommitWritesNothing` | store: the transaction |
| `TestValidIDRejectsPaths` | store: id checks |
| `TestIsWholeID` | store: id checks |
| `TestAmend` | store: the fields an amend stores. What an amend changes is in `amend.json` |
| `TestAppendToMissingCase` | store: a missing case is `fs.ErrNotExist` |
| `TestConcurrentAppendsTakeDistinctSequenceNumbers` | concurrency |
| `TestTwoWritersOnOneFileSerialise` | concurrency |
| `TestTwoProcessesOnOneFileSerialise` | concurrency |
| `TestWriterProcess` | concurrency: the writer process the test above starts |
| `TestAtRevisionIsCheckedInsideTheWriteTransaction` | concurrency |
| `TestEventsAreStampedOnceTheWriteHoldsTheLock` | concurrency |
| `TestWrittenRecordShape` | store: the bytes a write stores and the `at` column. The vectors compare records as values |

By reason: store 32, concurrency 7, text 7, Go API 2, core unreachable 2, core not yet vectorised 1.

## cmd/cases: 158 tests, all CLI

Every test in `cmd/cases` runs the CLI: it parses flags, builds records, prints output and sets exit codes. Where a command reaches a core rule, the rule is in the vectors; the tests stay for the CLI around it. A few CLI rules are stricter than the core, such as `amend` refusing an empty `--body` beside a `--link`, which the store would take as no change, and `--row` refusing a field the record does not have.

| File | Tests | Reason |
| --- | --- | --- |
| `amend_test.go` | `TestAmendRoundTrip`, `TestShowPrintsWhatAnAmendReplaced`, `TestAmendAtRevision`, `TestAmendInlineBody`, `TestAmendAddsOptions`, `TestAmendAddsLabels`, `TestAmendRefusals` | CLI: `amend` flags and output. Rules in `amend.json`, `writer.json` |
| `answer_test.go` | `TestAnswerByKind`, `TestAnswerTextFile`, `TestAnswerAtRevision`, `TestAnswerParkAtRevision`, `TestAnswerTakesPartOfAnID`, `TestNameConfigStampsTheHumanOnAnswer`, `TestBlankWorkerRecordsNoActor` | CLI: `answer` flags, ids and actors. Rules in `answer.json`, `writer.json` |
| `close_test.go` | `TestClose`, `TestCloseAtRevision`, `TestCloseInlineOutcome` | CLI: `close`. Rules in `transitions.json`, `writer.json` |
| `note_test.go` | `TestNoteReopensAnAnsweredCase`, `TestNoteAtRevision`, `TestNoteInlineBody` | CLI: `note`. Rules in `transitions.json`, `writer.json` |
| `open_test.go` | `TestOpenDecisionKeepsCommasInOptions`, `TestOpenBody`, `TestOpenInlineBody`, `TestOpenApprovalRows`, `TestOpenLabels`, `TestOpenWorkerAndLabelFromTheEnvironment`, `TestOpenRefusals` | CLI: `open` flags. Rules in `open.json` |
| `pickup_test.go` | `TestPickupAtRevision`, `TestPickup` | CLI: `pickup`. Rules in `transitions.json`, `writer.json` |
| `resume_test.go` | `TestResume`, `TestResumeAtRevision`, `TestResumeTakesPartOfAnID` | CLI: `resume`. Rules in `transitions.json`, `writer.json` |
| `withdraw_test.go` | `TestWithdraw`, `TestWithdrawAtRevision`, `TestWithdrawReason` | CLI: `withdraw`. Rules in `transitions.json`, `writer.json` |
| `e2e_test.go` | `TestDecisionEndToEnd` | CLI: open, answer, wait, pickup and close run as commands |
| `main_test.go` | `TestStoreDefaultsToDataHome`, `TestListOnMissingStoreIsEmpty`, `TestStoreFromEnvironment`, `TestVersionString`, `TestReportExitStatus`, `TestFindCaseTakesPartOfAnID` | CLI: the store path, version, exit codes and id prefixes |
| `config_test.go` | `TestConfigKeysMatchFlags`, `TestConfigSetsStore`, `TestConfigExpandsHome`, `TestConfigSetsListen`, `TestConfigSetsNoOpen`, `TestConfigFlagAndEnv`, `TestConfigFlagLastWins`, `TestBrokenConfigStopsCommandsButNotDiagnosis`, `TestConfigShow`, `TestConfigInit` | CLI: the config file |
| `list_test.go` | `TestListOrderFilterAndJSON`, `TestListShowsLabels`, `TestListByLabelAndWorker`, `TestListReportsBrokenCaseAndListsTheRest`, `TestListDefaultsToOpenAndParked`, `TestListByKindAndUrgency`, `TestWaitByKind`, `TestListCount`, `TestListOlderThanReadsTheLastEvent` | CLI: listing, filters and output |
| `show_test.go` | `TestShow`, `TestShowQuestionReply`, `TestShowRefusesPathsAndMissingCases`, `TestShowJSONRevisionCountsSkippedFiles`, `TestShowTakesPartOfAnID`, `TestShowReportsAnUnknownEventAsVersionSkew`, `TestShowURLOfTheRunningInbox`, `TestShowWarnsOnADamagedInstanceFile`, `TestShowAnswer` | CLI: `show` output. Revision and skipped rows are in `fold.json` |
| `wait_test.go` | `TestWaitReturnsAnAnswerThatLands`, `TestWaitTimesOutWithExitTwo`, `TestWaitIgnoresEventsFromBeforeItStarted`, `TestWaitSinceCountsEarlierEvents`, `TestWaitParkAndResume`, `TestWaitOnSpecificCases`, `TestWaitByLabelAndWorker`, `TestWaitByIDAndLabel`, `TestWaitForAStoreThatDoesNotExistYet`, `TestWaitReportsAnAnswerWithoutTimestamp`, `TestWaitWarnsOnceAboutAMalformedAnswer`, `TestWaitForHuman`, `TestWaitForHumanAmendAndResume`, `TestWaitSinceCaseID`, `TestWaitPrintsTheNextSince`, `TestWaitMarksWhichCasesAreFresh`, `TestWaitPickupPicksUpTheAnswer`, `TestWaitPickupLeavesParkedAndResumedCases`, `TestWaitPickupSkipsACaseChangedSinceItWasRead`, `TestWaitPickupByTwoWaitsPicksUpOnce`, `TestWaitPickupByLabel`, `TestWaitPickupFlags` | CLI: `wait`, which polls the store |
| `sweep_test.go` | `TestSweepDryRunWritesNothing`, `TestSweepWithdrawsOpenCasesAndLeavesTheRest`, `TestSweepFilters`, `TestSweepLeavesACaseChangedSinceItWasListed`, `TestSweepByKind`, `TestSweepWithdrawsThroughTheStore` | CLI: `sweep` |
| `prune_test.go` | `TestPruneDryRunWritesNothing`, `TestPruneArchives`, `TestPruneRefusesAnIDInTheArchive`, `TestPruneDelete`, `TestPruneSkipsABrokenCase`, `TestPruneRefusesOtherStates`, `TestPruneOnMissingStore`, `TestConfigSetsPruneAge`, `TestPruneAgeZeroTakesAFutureCase` | CLI: `prune` and the archive |
| `serve_test.go` | `TestServeRefusesNonLoopback`, `TestServeAnswersAndStops`, `TestServePushesChangesAndStopsWithAStreamOpen`, `TestServeWithoutTerminalPrintsURLAndOpensNothing`, `TestBrowserCommand`, `TestServeScreenStats`, `TestServeFailsWhenAddressIsTaken`, `TestServeWithoutStateDirectoryStillServes`, `TestServeQueuesNotificationsWithNoTabOpen` | CLI: `serve` |
| `serve_screen_test.go` | `TestScreenDrawRedrawsInPlace`, `TestScreenDrawStoreStates`, `TestScreenLogWriter`, `TestScreenLogWriterWithoutTee`, `TestScreenLogWriterStderrFails`, `TestScreenRunWithoutTerminal`, `TestReadKeys`, `TestUptimeAndAgo` | CLI: the `serve` screen |
| `serve_unix_test.go` | `TestServeStopsOnSignal` | CLI: signals |
| `status_test.go` | `TestServeRecordsInstanceForStatus`, `TestStatusWithoutServe`, `TestStaleInstanceIsNotRunning`, `TestSecondServeOnSameStoreIsRefused` | CLI: the instance file |
| `inbox_test.go` | `TestInboxPrintsTheRunningURL`, `TestInboxPrintsACasesPage`, `TestInboxPrintUnknownCase`, `TestInboxOpensTheBrowserWhenNotPrinting`, `TestInboxWithoutServe` | CLI: `inbox` |
| `service_test.go` | `TestServiceInstallWritesUnit`, `TestServiceInstallWritesPlist`, `TestServiceInstallTakesConfig`, `TestServiceInstallRefusesNonLoopback`, `TestServiceUninstall`, `TestServiceManagerDown`, `TestServiceInstallWithoutManager`, `TestServiceInstallRefusesWhileServeRuns`, `TestServiceInstallRefusesTakenAddress`, `TestServiceReinstallWhileServiceRuns`, `TestServiceReinstallChecksWhatChanged`, `TestServiceStatusNotInstalled`, `TestServiceStatusHandStartedServe`, `TestServiceStatusRunning`, `TestServiceStatusNotLoaded`, `TestServiceStatusNotAnswering`, `TestServiceStatusLoadedWithoutFile`, `TestServiceStatusOtherStoreAndBinary` | CLI: `service` |
| `skill_test.go` | `TestSkillShowPrintsFrontmatter`, `TestSkillInstallListUninstall`, `TestSkillCheck`, `TestSkillUnreadable`, `TestSkillUnknownAgent`, `TestSkillHelpNamesEveryAgent` | CLI: `skill` |

## internal/store tests the vectors carry: 22 tests

These tests' rules are all in the vectors. They stay, as Go's own checks.

| Test | Vectors |
| --- | --- |
| `TestTransitionMatrix` | `transitions.json`: every event from every state |
| `TestTransitions` | `transitions.json`, and `fold.json` for an unknown event |
| `TestReopenClearsAnswerAndPickup` | `transitions.json` |
| `TestEventsRecordTheStateBeforeThem` | `fold.json` |
| `TestOpenValidation` | `open.json` |
| `TestAnswerValidation` | `answer.json` |
| `TestAmendValidation` | `amend.json` |
| `TestAmendChangesTheCase` | `amend.json` |
| `TestAmendAddsLabels` | `amend.json` |
| `TestAmendSeq` | `amend.json` |
| `TestAnswerIsCheckedAgainstTheAmendedCase` | `answer.json` |
| `TestActorAndForFold` | `fold.json` |
| `TestActorAndForValidation` | `open.json`, `fold.json` |
| `TestActorFromALaterBuildFolds` | `fold.json` |
| `TestLoadToleratesMalformedRows` | `fold.json`, `writer.json` |
| `TestLoadPreservesUnknownFields` | `fold.json` |
| `TestLoadWithoutOpenFails` | `fold.json` |
| `TestOpenRecordWithoutLabelsLoads` | `fold.json` |
| `TestAppendNumbersEventsInOrder` | `writer.json` |
| `TestInvalidTransitionIsNotWritten` | `transitions.json`, `amend.json`, `answer.json` |
| `TestAtRevision` | `writer.json` |
| `TestAgentWritesAtRevision` | `writer.json` |
