//! The order of the checks. Vectors compare errors by category, and most of
//! these rules are `invalid`, so a vector cannot tell which of two faults was
//! reported first. Each test here gives a record two faults, one for each of
//! two checks next to each other in the order, and pins the message of the
//! first.

use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use super::{check_amend, check_answer, check_open};
use crate::case::{Append, Case};
use crate::error::Category;
use crate::names::Author;
use crate::record::{Amend, Answer, ApprovalRow, Open, Record, RowVerdict};

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| (*v).to_string()).collect()
}

fn row(id: &str) -> ApprovalRow {
    ApprovalRow {
        id: id.into(),
        label: "Label".into(),
        script: "make check".into(),
        link: "https://example.com/r".into(),
        ..ApprovalRow::default()
    }
}

fn author(kind: &str) -> Author {
    if kind == "message" {
        Author::Human
    } else {
        Author::Agent
    }
}

/// A valid open of `kind`, changed by `change`.
fn open(kind: &str, change: impl FnOnce(&mut Open)) -> Open {
    let mut open = Open {
        kind: kind.into(),
        urgency: "today".into(),
        title: "Title".into(),
        ..Open::default()
    };
    match kind {
        "decision" => open.options = strings(&["a", "b"]),
        "approval" => open.rows = vec![row("r1")],
        "briefing" => open.body = "Body".into(),
        "message" => open.worker = "w1".into(),
        _ => {}
    }
    change(&mut open);
    open
}

/// The message of the error a write of `open` gives.
fn open_error(open: &Open) -> String {
    check_open(open, author(&open.kind), true)
        .unwrap_err()
        .to_string()
}

/// A case opened as `open` of `kind`, changed by `change`.
fn case(kind: &str, change: impl FnOnce(&mut Open)) -> Case {
    let open = open(kind, change);
    let append = Append::new(author(kind), &Record::Open(open));
    let now = "2026-09-15T09:01:00Z".parse().unwrap();
    Case::open(&append, now, None).unwrap().0
}

fn amend_error(amend: &Amend, case: &Case) -> String {
    check_amend(amend, case).unwrap_err().to_string()
}

fn answer_error(answer: &Answer, case: &Case) -> String {
    check_answer(answer, case).unwrap_err().to_string()
}

#[test]
fn an_open_checks_its_kind_then_its_author_then_its_urgency_then_the_rest() {
    let both = open("decision", |o| o.urgency = "someday".into());
    let error = check_open(&both, Author::Human, true).unwrap_err();
    assert_eq!(error.category(), Category::Author);
    let both = open("decision", |o| {
        o.urgency = "someday".into();
        o.title = " ".into();
    });
    let error = check_open(&both, Author::Agent, true).unwrap_err();
    assert_eq!(error.category(), Category::Unknown);
}

#[test]
fn an_open_checks_its_title_then_a_briefing_body() {
    let both = open("briefing", |o| {
        o.title = " ".into();
        o.body = " ".into();
    });
    assert_eq!(open_error(&both), "the title is blank");
}

#[test]
fn an_open_checks_a_briefing_body_then_the_kind_fields() {
    let both = open("briefing", |o| {
        o.body = String::new();
        o.options = strings(&["a"]);
    });
    assert_eq!(open_error(&both), "a briefing needs a body");
}

#[test]
fn an_open_checks_options_then_rows_then_the_briefing_fields() {
    let both = open("fyi", |o| {
        o.options = strings(&["a"]);
        o.rows = vec![row("r1")];
    });
    assert_eq!(open_error(&both), "a fyi has no options");
    let both = open("fyi", |o| {
        o.rows = vec![row("r1")];
        o.landed = true;
    });
    assert_eq!(open_error(&both), "a fyi has no rows");
}

#[test]
fn an_open_checks_the_briefing_fields_then_what_its_kind_needs() {
    let both = open("decision", |o| {
        o.options = Vec::new();
        o.landed = true;
    });
    assert_eq!(open_error(&both), "a decision has no landed");
    let both = open("message", |o| {
        o.worker = String::new();
        o.left_out = strings(&["x"]);
    });
    assert_eq!(open_error(&both), "a message has no left_out");
}

#[test]
fn an_open_checks_what_its_kind_needs_then_its_for_re_and_labels() {
    let both = open("message", |o| {
        o.worker = " ".into();
        o.for_ = " ".into();
    });
    assert_eq!(open_error(&both), "a message goes to a worker or a label");
    let both = open("approval", |o| {
        o.rows = Vec::new();
        o.re = " ".into();
    });
    assert_eq!(open_error(&both), "an approval needs at least one row");
}

#[test]
fn an_open_checks_a_blank_option_then_the_recommend() {
    let both = open("decision", |o| {
        o.options = strings(&["a", " "]);
        o.recommend = 5;
    });
    assert_eq!(open_error(&both), "option 2 is blank");
}

#[test]
fn an_open_checks_the_recommend_then_the_rows() {
    let both = open("approval", |o| {
        o.recommend = 1;
        o.rows = vec![row("bad id")];
    });
    assert_eq!(
        open_error(&both),
        "only a decision takes a recommend, and the case's kind is approval"
    );
}

#[test]
fn an_open_checks_its_rows_then_for() {
    let both = open("approval", |o| {
        o.rows = vec![row("bad id")];
        o.for_ = " ".into();
    });
    assert_eq!(
        open_error(&both),
        "row 1: the id \"bad id\" is not letters, digits, '.', '-' and '_'"
    );
}

#[test]
fn an_open_checks_for_then_re_then_its_labels() {
    let both = open("fyi", |o| {
        o.for_ = " ".into();
        o.re = " ".into();
    });
    assert_eq!(open_error(&both), "for is blank");
    let both = open("fyi", |o| {
        o.re = " ".into();
        o.labels = strings(&[" "]);
    });
    assert_eq!(open_error(&both), "re is blank");
}

#[test]
fn an_open_checks_its_labels_then_the_rules_only_a_write_applies() {
    let both = open("decision", |o| {
        o.labels = strings(&[" "]);
        o.options = strings(&["a", "a"]);
    });
    assert_eq!(open_error(&both), "label 1 is blank");
    // The second fault alone is a write's: a replay takes it.
    let write_only = open("decision", |o| o.options = strings(&["a", "a"]));
    assert!(check_open(&write_only, Author::Agent, false).is_ok());
    assert_eq!(open_error(&write_only), "the option \"a\" is given twice");
}

#[test]
fn a_write_checks_new_options_then_links_then_worker_brief_and_context() {
    let both = open("decision", |o| {
        o.options = strings(&["a", "a"]);
        o.links = strings(&[" "]);
    });
    assert_eq!(open_error(&both), "the option \"a\" is given twice");
    let both = open("fyi", |o| {
        o.links = strings(&[" ", "x", "x"]);
    });
    assert_eq!(open_error(&both), "link 1 is blank");
    let both = open("fyi", |o| {
        o.links = strings(&["x", "x"]);
        o.worker = " ".into();
    });
    assert_eq!(open_error(&both), "the link \"x\" is given twice");
    let both = open("fyi", |o| {
        o.worker = " ".into();
        o.brief = " ".into();
    });
    assert_eq!(open_error(&both), "the worker is blank");
    let both = open("fyi", |o| {
        o.brief = " ".into();
        o.context = " ".into();
    });
    assert_eq!(open_error(&both), "the brief is blank");
}

#[test]
fn an_amend_checks_options_then_rows_then_the_briefing_fields() {
    let fyi = case("fyi", |_| {});
    let both = Amend {
        options: strings(&["a"]),
        rows: vec![row("r1")],
        ..Amend::default()
    };
    assert_eq!(amend_error(&both, &fyi), "a fyi has no options");
    let both = Amend {
        rows: vec![row("r1")],
        landed: Some(true),
        ..Amend::default()
    };
    assert_eq!(amend_error(&both, &fyi), "a fyi has no rows");
}

#[test]
fn an_amend_checks_the_briefing_fields_then_its_text() {
    let both = Amend {
        landed: Some(true),
        title: " ".into(),
        ..Amend::default()
    };
    assert_eq!(
        amend_error(&both, &case("fyi", |_| {})),
        "a fyi has no landed"
    );
}

#[test]
fn an_amend_checks_its_title_then_body_then_context_then_options() {
    let decision = case("decision", |_| {});
    let both = Amend {
        title: " ".into(),
        body: " ".into(),
        ..Amend::default()
    };
    assert_eq!(amend_error(&both, &decision), "the title is blank");
    let both = Amend {
        body: " ".into(),
        context: " ".into(),
        ..Amend::default()
    };
    assert_eq!(amend_error(&both, &decision), "the body is blank");
    let both = Amend {
        context: " ".into(),
        options: strings(&[" "]),
        ..Amend::default()
    };
    assert_eq!(amend_error(&both, &decision), "the context is blank");
}

#[test]
fn an_amend_checks_a_blank_option_or_its_rows_then_a_blank_link() {
    let both = Amend {
        options: strings(&[" "]),
        links: strings(&[" "]),
        ..Amend::default()
    };
    assert_eq!(
        amend_error(&both, &case("decision", |_| {})),
        "option 1 is blank"
    );
    let both = Amend {
        rows: vec![row("r1")],
        links: strings(&[" "]),
        ..Amend::default()
    };
    assert_eq!(
        amend_error(&both, &case("approval", |_| {})),
        "the row id \"r1\" is not unique in the case"
    );
}

#[test]
fn an_amend_checks_a_blank_link_then_its_options_against_the_case() {
    let both = Amend {
        links: strings(&[" "]),
        options: strings(&["a"]),
        ..Amend::default()
    };
    assert_eq!(
        amend_error(&both, &case("decision", |_| {})),
        "link 1 is blank"
    );
}

#[test]
fn an_amend_checks_new_options_then_the_recommend_then_new_links() {
    let decision = case("decision", |o| o.links = strings(&["x"]));
    let both = Amend {
        options: strings(&["a"]),
        recommend: 9,
        ..Amend::default()
    };
    assert_eq!(
        amend_error(&both, &decision),
        "the case has the option \"a\" already"
    );
    let both = Amend {
        recommend: 9,
        links: strings(&["x"]),
        ..Amend::default()
    };
    assert_eq!(
        amend_error(&both, &decision),
        "recommend 9 is not one of the 2 options"
    );
}

#[test]
fn an_amend_checks_new_links_then_dropped_links_then_labels() {
    let fyi = case("fyi", |o| o.labels = strings(&["l"]));
    let both = Amend {
        links: strings(&["x", "x"]),
        drop_links: strings(&["y"]),
        ..Amend::default()
    };
    assert_eq!(amend_error(&both, &fyi), "the link \"x\" is given twice");
    let both = Amend {
        drop_links: strings(&["y"]),
        labels: strings(&["l"]),
        ..Amend::default()
    };
    assert_eq!(
        amend_error(&both, &fyi),
        "the case has no link \"y\" to drop"
    );
}

#[test]
fn an_amend_checks_its_labels_against_the_case_before_whether_it_changes_anything() {
    let fyi = case("fyi", |o| o.labels = strings(&["l"]));
    let both = Amend {
        labels: strings(&["l"]),
        title: "Title".into(),
        ..Amend::default()
    };
    assert_eq!(
        amend_error(&both, &fyi),
        "the case has the label \"l\" already"
    );
}

#[test]
fn an_amend_changes_nothing_when_each_value_it_sets_is_the_cases() {
    let decision = case("decision", |o| {
        o.context = "Context".into();
        o.recommend = 1;
        o.links = strings(&["x"]);
    });
    let briefing = case("briefing", |o| o.left_out = strings(&["item"]));
    let nothing = [
        (Amend::default(), &decision),
        (
            Amend {
                reopen: true,
                ..Amend::default()
            },
            &decision,
        ),
        (
            Amend {
                title: "Title".into(),
                context: "Context".into(),
                recommend: 1,
                ..Amend::default()
            },
            &decision,
        ),
        (
            Amend {
                body: "Body".into(),
                landed: Some(false),
                left_out: strings(&["item"]),
                ..Amend::default()
            },
            &briefing,
        ),
    ];
    for (amend, case) in nothing {
        assert!(
            amend_error(&amend, case).starts_with("the amend changes nothing"),
            "{amend:?}"
        );
    }
}

#[test]
fn an_amend_changes_something_when_any_one_value_differs() {
    let decision = case("decision", |o| {
        o.context = "Context".into();
        o.recommend = 1;
        o.links = strings(&["x"]);
    });
    let briefing = case("briefing", |o| o.left_out = strings(&["item"]));
    let something = [
        (
            Amend {
                options: strings(&["c"]),
                ..Amend::default()
            },
            &decision,
        ),
        (
            Amend {
                links: strings(&["y"]),
                ..Amend::default()
            },
            &decision,
        ),
        (
            Amend {
                drop_links: strings(&["x"]),
                ..Amend::default()
            },
            &decision,
        ),
        (
            Amend {
                labels: strings(&["l"]),
                ..Amend::default()
            },
            &decision,
        ),
        (
            Amend {
                title: "Other".into(),
                ..Amend::default()
            },
            &decision,
        ),
        (
            Amend {
                context: "Other".into(),
                ..Amend::default()
            },
            &decision,
        ),
        (
            Amend {
                recommend: 2,
                ..Amend::default()
            },
            &decision,
        ),
        (
            Amend {
                body: "Other".into(),
                ..Amend::default()
            },
            &briefing,
        ),
        (
            Amend {
                landed: Some(true),
                ..Amend::default()
            },
            &briefing,
        ),
        (
            Amend {
                left_out: strings(&["other"]),
                ..Amend::default()
            },
            &briefing,
        ),
    ];
    for (amend, case) in something {
        assert_eq!(check_amend(&amend, case), Ok(()), "{amend:?}");
    }
}

#[test]
fn an_answer_checks_the_responses_in_their_order_then_none() {
    let decision = case("decision", |_| {});
    let both = Answer {
        signoff: "accept".into(),
        text: "t".into(),
        ..Answer::default()
    };
    assert_eq!(
        answer_error(&both, &decision),
        "a decision answer does not take signoff"
    );
    assert_eq!(
        answer_error(&Answer::default(), &decision),
        "the answer has no decision response"
    );
}

#[test]
fn an_answer_checks_its_responses_then_file() {
    let both = Answer {
        ack: true,
        file: vec![1],
        ..Answer::default()
    };
    assert_eq!(
        answer_error(&both, &case("decision", |_| {})),
        "a decision answer does not take ack"
    );
}

#[test]
fn an_answer_checks_file_then_its_amend_seq() {
    let both = Answer {
        drop: true,
        file: vec![1],
        amend_seq: 5,
        ..Answer::default()
    };
    assert_eq!(
        answer_error(&both, &case("briefing", |o| o.left_out = strings(&["i"]))),
        "file goes only with a briefing's verdict, not with drop or on another kind"
    );
}

#[test]
fn an_answer_checks_its_amend_seq_then_a_drop_with_a_response() {
    let both = Answer {
        drop: true,
        choice: 1,
        amend_seq: 5,
        ..Answer::default()
    };
    assert_eq!(
        answer_error(&both, &case("decision", |_| {})),
        "the answer saw amend 5, and the case's latest is 0"
    );
}

#[test]
fn an_answer_checks_a_drop_with_a_response_then_the_kind_rules() {
    let decision = case("decision", |_| {});
    let both = Answer {
        drop: true,
        choice: 9,
        ..Answer::default()
    };
    assert_eq!(
        answer_error(&both, &decision),
        "an answer drops the case or answers it, not both"
    );
    // A drop alone skips the kind's rules.
    let drop = Answer {
        drop: true,
        ..Answer::default()
    };
    assert_eq!(check_answer(&drop, &decision), Ok(()));
}

#[test]
fn an_answer_checks_the_rules_of_the_cases_kind() {
    let approval = case("approval", |_| {});
    let unknown_row = Answer {
        rows: vec![RowVerdict {
            id: "r9".into(),
            verdict: "approve".into(),
            ..RowVerdict::default()
        }],
        ..Answer::default()
    };
    assert_eq!(
        answer_error(&unknown_row, &approval),
        "the case has no row \"r9\""
    );
    let blank = Answer {
        text: " ".into(),
        ..Answer::default()
    };
    assert_eq!(
        answer_error(&blank, &case("question", |_| {})),
        "the text is blank"
    );
    let changes = Answer {
        signoff: "changes".into(),
        ..Answer::default()
    };
    assert_eq!(
        answer_error(&changes, &case("signoff", |_| {})),
        "asking for changes needs a note"
    );
    let send_back = Answer {
        briefing: "send_back".into(),
        ..Answer::default()
    };
    assert_eq!(
        answer_error(&send_back, &case("briefing", |_| {})),
        "sending back needs a reason"
    );
}
