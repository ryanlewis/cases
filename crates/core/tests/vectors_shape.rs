//! Checks that every file in `spec/vectors/` keeps the format its README
//! sets out, and that the checks themselves refuse what they should.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a test panics to fail; clippy exempts #[test] functions but not the helpers here"
)]

mod common;

use std::fs;

#[test]
fn every_vector_file_has_the_documented_shape() {
    if let Err(errors) = common::load_all() {
        panic!("{} problems:\n{}", errors.len(), errors.join("\n"));
    }
    if let Err(errors) = common::load_stats() {
        panic!("{} problems:\n{}", errors.len(), errors.join("\n"));
    }
}

/// The files and their number of vectors, as a table in `doc` gives them in
/// its column `column`: the case vectors' files, or with `store` those of
/// the store vectors, `store/…`. A row whose cell there is not a number is
/// left out.
fn counts_in(doc: &str, column: usize, store: bool) -> Vec<(String, usize)> {
    let text = fs::read_to_string(common::vector_dir().join(doc)).expect("the doc reads");
    let mut listed: Vec<(String, usize)> = text
        .lines()
        .filter_map(|line| {
            let cells: Vec<&str> = line.split('|').map(str::trim).collect();
            let name = cells.get(1)?.strip_prefix('`')?.strip_suffix(".json`")?;
            let name = match name.strip_prefix("store/") {
                Some(name) if store => name,
                None if !store => name,
                _ => return None,
            };
            Some((name.to_owned(), cells.get(column)?.parse().ok()?))
        })
        .collect();
    listed.sort();
    listed
}

/// The README's table of files, and the counts in CHANGES.md, give each
/// file's name and number of vectors.
#[test]
fn the_readme_counts_the_vectors_in_each_file() {
    let files = common::load_all().expect("the vectors load");
    let stats = common::load_stats().expect("the stats vectors load");
    let mut found: Vec<(String, usize)> = files
        .iter()
        .map(|f| (f.name.clone(), f.vectors.len()))
        .chain([(common::STATS.to_owned(), stats.len())])
        .collect();
    found.sort();
    assert_eq!(
        counts_in("README.md", 2, false),
        found,
        "the README's table of files, against the files"
    );
    assert_eq!(
        counts_in("CHANGES.md", 3, false),
        found,
        "the column Now of CHANGES.md's counts, against the files"
    );
}

/// The same for the store vectors in `store/`, which `cases-vectors` loads.
#[test]
fn the_readme_counts_the_store_vectors_in_each_file() {
    let files = cases_vectors::load_all().expect("the store vectors load");
    let found: Vec<(String, usize)> = files
        .iter()
        .map(|(name, vectors)| (name.clone(), vectors.len()))
        .collect();
    assert!(!found.is_empty());
    assert_eq!(
        counts_in("README.md", 2, true),
        found,
        "the README's table of store files, against the files"
    );
    assert_eq!(
        counts_in("CHANGES.md", 3, true),
        found,
        "the column Now of CHANGES.md's counts, against the store files"
    );
}

fn refused(text: &str, why: &str) {
    match common::parse("test", text) {
        Ok(_) => panic!("accepted {text}"),
        Err(errors) => assert!(
            errors.iter().any(|e| e.contains(why)),
            "refused without {why:?}: {errors:?}"
        ),
    }
}

const OPEN_ROW: &str = r#"{"seq": 1, "author": "agent", "event": "open", "data": {"kind": "fyi"}}"#;

#[test]
fn a_file_that_is_not_one_list_is_refused() {
    refused(r#"{"description": "x"}"#, "not one list");
    refused("[] []", "trailing characters");
    refused("[]", "no vectors");
}

#[test]
fn a_key_given_twice_is_refused_at_any_depth() {
    refused(
        r#"[{"description": "x", "description": "y", "events": [], "expect": {}}]"#,
        "given twice",
    );
    refused(
        r#"[{"description": "x", "events": [{"seq": 1, "author": "agent", "event": "open", "data": {"kind": "fyi", "kind": "stuck"}}], "expect": {}}]"#,
        "given twice",
    );
}

#[test]
fn a_key_the_format_does_not_have_is_refused() {
    refused(
        r#"[{"description": "x", "events": [], "expected": {}}]"#,
        "unknown field",
    );
    refused(
        r#"[{"description": "x", "events": [], "expect": {"State": "open"}}]"#,
        "not a property",
    );
    refused(
        &format!(
            r#"[{{"description": "x", "events": [{OPEN_ROW}], "expect": {{"problems": [{{"seq": 1, "why": "x"}}]}}}}]"#
        ),
        "not a property of an element",
    );
}

#[test]
fn a_vector_needs_a_unique_description_and_one_expectation() {
    refused(
        r#"[{"description": "x", "events": [], "expect": {}}, {"description": "x", "events": [], "expect": {}}]"#,
        "not unique",
    );
    refused(
        r#"[{"description": "x", "events": []}]"#,
        "one of expect and expect_error",
    );
    refused(
        r#"[{"description": "x", "events": [], "expect": null}]"#,
        "is null",
    );
}

#[test]
fn a_replay_fails_only_with_no_open() {
    refused(
        &format!(
            r#"[{{"description": "x", "events": [{OPEN_ROW}], "expect_error": {{"category": "malformed"}}}}]"#
        ),
        "fails only with no-open",
    );
}

#[test]
fn rows_need_a_rising_seq_and_one_record() {
    refused(
        &format!(r#"[{{"description": "x", "events": [{OPEN_ROW}, {OPEN_ROW}], "expect": {{}}}}]"#),
        "does not rise",
    );
    refused(
        r#"[{"description": "x", "events": [{"seq": 1, "author": "agent", "event": "open", "data": {}, "raw": "{}"}], "expect": {}}]"#,
        "one of data and raw",
    );
}

#[test]
fn an_error_category_must_be_one_of_the_documented_ones() {
    refused(
        r#"[{"description": "x", "events": [], "expect_error": {"category": "conflict"}}]"#,
        "not a category",
    );
    refused(
        &format!(
            r#"[{{"description": "x", "events": [{OPEN_ROW}], "expect": {{"problems": [{{"seq": 1, "category": "bad"}}]}}}}]"#
        ),
        "does not fit",
    );
}

#[test]
fn an_appended_record_names_only_its_event_s_fields() {
    let append = |data: &str| {
        format!(
            r#"[{{"description": "x", "events": [{OPEN_ROW}], "append": {{"author": "human", "event": "answer", "data": {data}, "now": "2026-09-15T09:02:00Z"}}, "expect": {{}}}}]"#
        )
    };
    refused(
        &append(r#"{"ack": true, "colour": "teal"}"#),
        "not a field of the record",
    );
    refused(&append(r#"{"ack": null}"#), "is not Flag");
    refused(&append(r#"{"choice": 1.0}"#), "is not Whole");
    refused(
        &append(r#"{"actor": {"name": "Ryan", "role": "human"}}"#),
        "not a field of the record",
    );
    refused(
        &append(r#"{"answered_at": "2026-09-15 09:02:00Z"}"#),
        "is not Time",
    );
    refused(
        r#"[{"description": "x", "events": [], "append": {"author": "agent", "event": "comment", "data": {}, "now": "2026-09-15T09:02:00Z"}, "expect_error": {"category": "unknown"}}]"#,
        "not an event a writer takes",
    );
}

#[test]
fn an_open_is_appended_only_to_a_case_with_no_rows() {
    refused(
        &format!(
            r#"[{{"description": "x", "events": [{OPEN_ROW}], "append": {{"author": "agent", "event": "open", "data": {{}}, "now": "2026-09-15T09:02:00Z"}}, "expect_error": {{"category": "transition"}}}}]"#
        ),
        "only to no events",
    );
}

#[test]
fn expected_times_are_in_utc_with_only_the_digits_they_need() {
    for time in [
        "2026-09-15T09:02:00+00:00",
        "2026-09-15T09:02:00.50Z",
        "2026-09-15t09:02:00Z",
    ] {
        refused(
            &format!(
                r#"[{{"description": "x", "events": [{OPEN_ROW}], "expect": {{"updated_at": "{time}"}}}}]"#
            ),
            "is not a value of Time",
        );
    }
}

#[test]
fn times_in_expected_records_are_in_utc() {
    refused(
        &format!(
            r#"[{{"description": "x", "events": [{OPEN_ROW}], "expect": {{"answer": {{"ack": true, "answered_at": "2026-09-15T11:02:00+02:00"}}}}}}]"#
        ),
        "is not Time",
    );
}

#[test]
fn an_expected_seq_names_a_row() {
    refused(
        &format!(
            r#"[{{"description": "x", "events": [{OPEN_ROW}], "expect": {{"events": [{{"seq": 2}}]}}}}]"#
        ),
        "does not fit",
    );
}

#[test]
fn an_expected_element_agrees_with_its_row() {
    for element in [
        r#"{"seq": 1, "file": "0001-agent-note.json"}"#,
        r#"{"seq": 1, "file": "1-agent-open.json"}"#,
        r#"{"seq": 1, "author": "human"}"#,
        r#"{"seq": 1, "event": "note"}"#,
    ] {
        refused(
            &format!(
                r#"[{{"description": "x", "events": [{OPEN_ROW}], "expect": {{"events": [{element}]}}}}]"#
            ),
            "does not fit",
        );
    }
    refused(
        &format!(
            r#"[{{"description": "x", "events": [{OPEN_ROW}], "append": {{"author": "agent", "event": "note", "data": {{}}, "now": "2026-09-15T09:02:00Z"}}, "expect": {{"events": [{{"seq": 1}}, {{"seq": 2, "file": "0002-agent-pickup.json"}}]}}}}]"#
        ),
        "does not fit",
    );
}

#[test]
fn an_expected_revision_counts_the_rows() {
    refused(
        &format!(
            r#"[{{"description": "x", "events": [{OPEN_ROW}], "expect": {{"revision": 2}}}}]"#
        ),
        "not the number of rows",
    );
    refused(
        &format!(
            r#"[{{"description": "x", "events": [{OPEN_ROW}], "expect": {{"events": [], "problems": []}}}}]"#
        ),
        "are not the 1 rows",
    );
}

#[test]
fn an_error_names_its_vector() {
    refused(
        r#"[{"description": "x", "events": [], "expect": {}}, {"description": "y", "events": [], "expected": {}}]"#,
        "test: vector 2: unknown field",
    );
}

#[test]
fn a_freshness_vector_is_drawn_once_and_expects_since() {
    let drawn = |keys: &str, expect: &str| {
        format!(r#"[{{"description": "x", "events": [{OPEN_ROW}], {keys}"expect": {expect}}}]"#)
    };
    refused(
        &drawn("", r#"{"since": "moved"}"#),
        "with no drawn_after or drawn_at",
    );
    refused(
        &drawn(r#""drawn_after": 1, "#, "{}"),
        "needs expect with since",
    );
    refused(
        &drawn(
            r#""drawn_after": 1, "drawn_at": {"revision": 1, "seq": 1}, "#,
            r#"{"since": "moved"}"#,
        ),
        "not both",
    );
    refused(
        &drawn(r#""drawn_after": 0, "#, r#"{"since": "moved"}"#),
        "is not from 1 to the 1 rows",
    );
    refused(
        &drawn(r#""drawn_after": 2, "#, r#"{"since": "moved"}"#),
        "is not from 1 to the 1 rows",
    );
    refused(
        &drawn(r#""drawn_after": 1, "#, r#"{"since": "stale"}"#),
        "is not a value of Since",
    );
    refused(
        &drawn(r#""drawn_at": {"revision": 1}, "#, r#"{"since": "moved"}"#),
        "missing field `seq`",
    );
    refused(
        &format!(
            r#"[{{"description": "x", "events": [{OPEN_ROW}], "drawn_after": 1, "append": {{"author": "agent", "event": "note", "data": {{}}, "now": "2026-09-15T09:02:00Z"}}, "expect": {{"since": "moved"}}}}]"#
        ),
        "has no append",
    );
}

fn refused_stats(text: &str, why: &str) {
    match common::parse_stats("test", text) {
        Ok(_) => panic!("accepted {text}"),
        Err(errors) => assert!(
            errors.iter().any(|e| e.contains(why)),
            "refused without {why:?}: {errors:?}"
        ),
    }
}

#[test]
fn a_stats_vector_counts_each_day_and_gives_a_median() {
    let vector = |cases: &str, now: &str, expect: &str| {
        format!(
            r#"[{{"description": "x", "cases": {cases}, "now": "{now}", "days": 2, "expect": {expect}}}]"#
        )
    };
    let fine = r#"{"opened": [0, 1], "answered": [0, 0], "median": null}"#;
    let now = "2026-11-02T12:00:00Z";
    assert!(common::parse_stats("test", &vector(&format!("[[{OPEN_ROW}]]"), now, fine)).is_ok());
    refused_stats(
        &vector(
            "[]",
            now,
            r#"{"opened": [0], "answered": [0, 0], "median": null}"#,
        ),
        "opened has 1 counts",
    );
    refused_stats(
        &vector(
            "[]",
            now,
            r#"{"opened": [0, 0], "answered": [0, 0, 0], "median": null}"#,
        ),
        "answered has 3 counts",
    );
    refused_stats(
        &vector("[]", now, r#"{"opened": [0, 0], "answered": [0, 0]}"#),
        "needs median",
    );
    refused_stats(
        &vector("[]", "2026-11-02T12:00:00+01:00", fine),
        "not a time in UTC",
    );
    refused_stats(
        &vector(
            r#"[[{"seq": 0, "author": "agent", "event": "open", "data": {}}]]"#,
            now,
            fine,
        ),
        "cases[0][0]: seq 0 is below 1",
    );
    refused_stats(
        &vector(
            r#"[[{"seq": 1, "author": "agent", "event": "open", "data": null}]]"#,
            now,
            fine,
        ),
        "cases[0][0]: \"data\" is null",
    );
    refused_stats(
        &format!(
            r#"[{{"description": "x", "cases": [], "now": null, "days": 2, "expect": {fine}}}]"#
        ),
        "\"now\" is null",
    );
    refused_stats(
        &format!(
            r#"[{{"description": "x", "cases": [], "now": "{now}", "days": 2, "expect": {fine}, "append": {{}}}}]"#
        ),
        "unknown field",
    );
    refused_stats("[]", "no vectors");
}
