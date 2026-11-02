//! What each event of a case said, as lines of text for its thread.

use cases_core::{Case, Event, Kind, Record, State};

use crate::output::Actors;

/// One line of what an event said. When an amend replaced the body or the
/// context, `previous` is the text the case had before, on the line that
/// says so, unless that was empty or only white space.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Line {
    pub(crate) text: String,
    pub(crate) previous: Option<Previous>,
}

/// The body or context an amend replaced.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Previous {
    Body(String),
    Context(String),
}

/// What each of the case's events said, in order: who wrote it, when the
/// actor is named, then what it did.
pub(crate) fn thread<'a>(case: &'a Case, actors: &Actors) -> Vec<(&'a Event, Vec<Line>)> {
    let mut title = "";
    let mut body = "";
    let mut context = "";
    let mut recommend = 0;
    let mut landed = false;
    let mut left_out: &[String] = &[];
    let mut thread = Vec::new();
    for event in case.events() {
        let mut lines = actor_line(event, actors);
        // A note or an amend of an answered or picked-up case reopened it.
        if matches!(event.record, Record::Note(_) | Record::Amend(_))
            && matches!(event.from, Some(State::Answered | State::PickedUp))
        {
            add(&mut lines, "reopened the case");
        }
        match &event.record {
            Record::Open(open) => {
                title = &open.title;
                body = &open.body;
                context = &open.context;
                recommend = open.recommend;
                landed = open.landed;
                left_out = &open.left_out;
                // A message says who it went to.
                if case.kind() == Kind::Message {
                    let to = crate::ctx::addressee(&open.worker, &open.labels);
                    if !to.is_empty() {
                        add(&mut lines, &format!("sent to {to}"));
                    }
                }
            }
            Record::Amend(amend) => {
                if !amend.title.is_empty() && amend.title != title {
                    add(&mut lines, &format!("replaced the title: {}", amend.title));
                    title = &amend.title;
                }
                if !amend.body.is_empty() && amend.body != body {
                    lines.extend(replaced("replaced the body", body, Previous::Body));
                    body = &amend.body;
                }
                for option in &amend.options {
                    add(&mut lines, &format!("added option: {option}"));
                }
                if amend.recommend != 0 && amend.recommend != recommend {
                    recommend = amend.recommend;
                    let option = option(case, recommend).unwrap_or_default();
                    add(&mut lines, &format!("recommended {recommend}. {option}"));
                }
                for row in &amend.rows {
                    add(&mut lines, &format!("added row [{}] {}", row.id, row.label));
                }
                if let Some(now) = amend.landed
                    && now != landed
                {
                    landed = now;
                    add(&mut lines, if now { "landed" } else { "not landed" });
                }
                if !amend.left_out.is_empty() && amend.left_out != left_out {
                    left_out = &amend.left_out;
                    let items = left_out.join("; ");
                    add(&mut lines, &format!("replaced the left-out items: {items}"));
                }
                for link in &amend.drop_links {
                    add(&mut lines, &format!("dropped link: {link}"));
                }
                for link in &amend.links {
                    add(&mut lines, &format!("added link: {link}"));
                }
                for label in &amend.labels {
                    add(&mut lines, &format!("added label: {label}"));
                }
                if !amend.context.is_empty() && amend.context != context {
                    let text = format!("replaced the context: {}", amend.context);
                    lines.extend(replaced(&text, context, Previous::Context));
                    context = &amend.context;
                }
            }
            record => {
                lines.extend(describe(case, record));
                // An answer files a briefing's items by their number in the
                // list it saw, which a later amend may have replaced.
                if let Record::Answer(answer) = record {
                    for n in &answer.file {
                        let item = usize::try_from(*n)
                            .ok()
                            .and_then(|n| left_out.get(n.checked_sub(1)?))
                            .map_or("", String::as_str);
                        add(&mut lines, &format!("file {n}. {item}"));
                    }
                }
            }
        }
        thread.push((event, lines));
    }
    thread
}

/// "by" and the actor's name, when the event has a named actor. A pickup
/// whose `by` names the same actor has no second line for it.
fn actor_line(event: &Event, actors: &Actors) -> Vec<Line> {
    let Some(name) = event.actor().and_then(|actor| actors.name(actor)) else {
        return Vec::new();
    };
    if let Record::Pickup(pickup) = &event.record
        && pickup.by == name
    {
        return Vec::new();
    }
    let mut lines = Vec::new();
    add(&mut lines, &format!("by {name}"));
    lines
}

/// The lines of `text`, the last one with the text the case had before, if
/// it had any.
fn replaced(text: &str, before: &str, previous: fn(String) -> Previous) -> Vec<Line> {
    let mut lines = Vec::new();
    add(&mut lines, text);
    if let Some(last) = lines.last_mut()
        && !before.trim().is_empty()
    {
        last.previous = Some(previous(before.to_owned()));
    }
    lines
}

/// What an event other than an open or an amend said.
fn describe(case: &Case, record: &Record) -> Vec<Line> {
    let mut lines = Vec::new();
    match record {
        Record::Answer(answer) => {
            if let Some(option) = option(case, answer.choice) {
                add(&mut lines, &format!("chose {}. {option}", answer.choice));
            } else if answer.other {
                add(&mut lines, "chose other");
            } else if !answer.signoff.is_empty() {
                add(&mut lines, &format!("signoff: {}", answer.signoff));
            } else if !answer.briefing.is_empty() {
                let verdict = answer.briefing.replace('_', " ");
                add(&mut lines, &format!("briefing: {verdict}"));
            } else if !answer.text.is_empty() && case.kind() == Kind::Question {
                add(&mut lines, &format!("reply: {}", answer.text));
            } else if !answer.text.is_empty() {
                add(&mut lines, &format!("guidance: {}", answer.text));
            } else if answer.drop {
                add(&mut lines, "drop");
            } else if answer.ack {
                add(&mut lines, "acknowledged");
            }
            for row in &answer.rows {
                // `reject` is stored as it is, and shown to people as "don't
                // run" (HU-01).
                let verdict = if row.verdict == "reject" {
                    "don't run"
                } else {
                    &row.verdict
                };
                if row.note.is_empty() {
                    add(&mut lines, &format!("[{}] {verdict}", row.id));
                } else {
                    add(&mut lines, &format!("[{}] {verdict}: {}", row.id, row.note));
                }
            }
            if !answer.note.is_empty() {
                add(&mut lines, &format!("note: {}", answer.note));
            }
        }
        Record::Pickup(pickup) if !pickup.by.is_empty() => {
            add(&mut lines, &format!("by {}", pickup.by));
        }
        Record::Note(note) => add(&mut lines, &note.body),
        Record::Close(close) => {
            add(&mut lines, &close.outcome);
            for link in &close.links {
                add(&mut lines, link);
            }
        }
        Record::Withdraw(withdraw) if !withdraw.reason.is_empty() => {
            add(&mut lines, &format!("reason: {}", withdraw.reason));
        }
        Record::Park(park) if !park.note.is_empty() => {
            add(&mut lines, &format!("note: {}", park.note));
        }
        _ => {}
    }
    lines
}

/// The case's option numbered `n` from 1, as a choice or a recommend names
/// it, if it has one.
fn option(case: &Case, n: i64) -> Option<&str> {
    let i = usize::try_from(n).ok()?.checked_sub(1)?;
    case.options().get(i).map(String::as_str)
}

/// Adds `text`, trimmed, one line to each of its lines.
fn add(lines: &mut Vec<Line>, text: &str) {
    lines.extend(text.trim().lines().map(|line| Line {
        text: line.to_owned(),
        previous: None,
    }));
}

#[cfg(test)]
mod tests {
    use cases_core::{ActorRow, Row};

    use super::*;

    const AGENT: &str = "AAAAAAAAAAAAAAAAAAAAAA";
    const HUMAN: &str = "BBBBBBBBBBBBBBBBBBBBBB";
    const NAMELESS: &str = "CCCCCCCCCCCCCCCCCCCCCC";
    const GONE: &str = "DDDDDDDDDDDDDDDDDDDDDD";

    fn actors() -> Actors {
        let row = |id: &str, kind: &str, name: Option<&str>| {
            ActorRow::new(
                id.to_owned(),
                kind.to_owned(),
                name.map(str::to_owned),
                "2026-01-01T00:00:00Z".to_owned(),
            )
        };
        Actors::new(vec![
            row(AGENT, "agent", Some("builder")),
            row(HUMAN, "human", Some("ryan")),
            row(NAMELESS, "agent", None),
        ])
    }

    /// A case folded from `(author, event, data)` rows, numbered from 1.
    fn fold(rows: &[(&str, &str, &str)]) -> Case {
        let rows: Vec<Row> = rows
            .iter()
            .zip(1..)
            .map(|(&(author, event, data), seq)| Row {
                seq,
                author: author.to_owned(),
                event: event.to_owned(),
                // Every open needs an urgency; the tests leave it out.
                data: if event == "open" {
                    data.replacen('{', r#"{"urgency":"today","#, 1)
                } else {
                    data.to_owned()
                },
            })
            .collect();
        let case = Case::fold(&rows).unwrap();
        assert!(case.problems().is_empty(), "{:?}", case.problems());
        case
    }

    fn texts(lines: &[Line]) -> Vec<&str> {
        lines.iter().map(|line| line.text.as_str()).collect()
    }

    /// The text of each event's lines, in order.
    fn thread_texts(case: &Case, actors: &Actors) -> Vec<Vec<String>> {
        thread(case, actors)
            .iter()
            .map(|(_, lines)| lines.iter().map(|line| line.text.clone()).collect())
            .collect()
    }

    fn strs(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|&line| line.to_owned()).collect()
    }

    const DECISION: &str =
        r#"{"kind":"decision","title":"Pick","options":["alpha","beta"],"recommend":2}"#;
    const QUESTION: &str = r#"{"kind":"question","title":"Q"}"#;

    /// What `describe` says of an answer `build` makes, to `case`.
    fn said(case: &Case, build: impl FnOnce(&mut cases_core::Answer)) -> Vec<String> {
        let mut answer = cases_core::Answer::default();
        build(&mut answer);
        texts(&describe(case, &Record::Answer(answer)))
            .into_iter()
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn a_choice_names_its_option() {
        let case = fold(&[("agent", "open", DECISION)]);
        assert_eq!(said(&case, |a| a.choice = 1), ["chose 1. alpha"]);
        assert_eq!(said(&case, |a| a.choice = 2), ["chose 2. beta"]);
        // A number the case has no option for says nothing.
        assert!(said(&case, |a| a.choice = 3).is_empty());
        assert!(said(&case, |a| a.choice = 0).is_empty());
        assert!(said(&case, |a| a.choice = -1).is_empty());
        assert_eq!(option(&case, 1), Some("alpha"));
        assert_eq!(option(&case, 2), Some("beta"));
        assert_eq!(option(&case, 3), None);
        assert_eq!(option(&case, 0), None);
        assert_eq!(option(&case, -5), None);
    }

    #[test]
    fn an_answer_says_the_first_of_its_forms() {
        let decision = fold(&[("agent", "open", DECISION)]);
        let question = fold(&[("agent", "open", QUESTION)]);
        // A choice beats everything after it.
        assert_eq!(
            said(&decision, |a| {
                a.choice = 1;
                a.other = true;
                a.signoff = "accept".into();
                a.text = "t".into();
                a.drop = true;
                a.ack = true;
            }),
            ["chose 1. alpha"]
        );
        assert_eq!(
            said(&decision, |a| {
                a.other = true;
                a.signoff = "accept".into();
                a.text = "t".into();
            }),
            ["chose other"]
        );
        assert_eq!(
            said(&decision, |a| {
                a.signoff = "accept".into();
                a.text = "t".into();
            }),
            ["signoff: accept"]
        );
        assert_eq!(
            said(&question, |a| a.text = "yes\nmaybe".into()),
            ["reply: yes", "maybe"]
        );
        assert_eq!(
            said(&decision, |a| {
                a.text = "do it".into();
                a.drop = true;
            }),
            ["guidance: do it"]
        );
        assert_eq!(
            said(&decision, |a| {
                a.drop = true;
                a.ack = true;
            }),
            ["drop"]
        );
        assert_eq!(said(&decision, |a| a.ack = true), ["acknowledged"]);
        assert!(said(&decision, |_| {}).is_empty());
    }

    #[test]
    fn an_answer_adds_row_verdicts_then_its_note() {
        let case = fold(&[("agent", "open", QUESTION)]);
        let row = |id: &str, verdict: &str, note: &str| {
            let mut row = cases_core::RowVerdict::default();
            row.id = id.into();
            row.verdict = verdict.into();
            row.note = note.into();
            row
        };
        let got = said(&case, |a| {
            a.rows = vec![
                row("a", "approve", ""),
                row("b", "reject", "too risky"),
                row("c", "hold", "later"),
                row("d", "reject", ""),
            ];
            a.note = "overall\nfine".into();
        });
        assert_eq!(
            got,
            [
                "[a] approve",
                "[b] don't run: too risky",
                "[c] hold: later",
                "[d] don't run",
                "note: overall",
                "fine"
            ]
        );
        // The verdicts follow the first form's line.
        let got = said(&case, |a| {
            a.ack = true;
            a.rows = vec![row("a", "approve", "")];
        });
        assert_eq!(got, ["acknowledged", "[a] approve"]);
    }

    #[test]
    fn an_answer_through_the_fold() {
        let case = fold(&[
            ("agent", "open", DECISION),
            ("human", "answer", r#"{"choice":2,"note":"ok"}"#),
        ]);
        assert_eq!(
            thread_texts(&case, &Actors::default()),
            vec![vec![], strs(&["chose 2. beta", "note: ok"])]
        );
    }

    #[test]
    fn events_without_text_have_no_lines() {
        let case = fold(&[("agent", "open", QUESTION)]);
        for record in [
            Record::Pickup(cases_core::Pickup::default()),
            Record::Withdraw(cases_core::Withdraw::default()),
            Record::Park(cases_core::Park::default()),
            Record::Resume(cases_core::Resume::default()),
        ] {
            assert!(describe(&case, &record).is_empty(), "{record:?}");
        }
    }

    #[test]
    fn park_withdraw_and_pickup_show_their_text() {
        let case = fold(&[("agent", "open", QUESTION)]);
        let mut pickup = cases_core::Pickup::default();
        pickup.by = "worker-1".into();
        let mut park = cases_core::Park::default();
        park.note = "later".into();
        let mut withdraw = cases_core::Withdraw::default();
        withdraw.reason = "moot".into();
        assert_eq!(
            texts(&describe(&case, &Record::Pickup(pickup))),
            ["by worker-1"]
        );
        assert_eq!(
            texts(&describe(&case, &Record::Park(park))),
            ["note: later"]
        );
        assert_eq!(
            texts(&describe(&case, &Record::Withdraw(withdraw))),
            ["reason: moot"]
        );
    }

    #[test]
    fn a_note_and_a_close_through_the_fold() {
        let case = fold(&[
            ("agent", "open", QUESTION),
            ("human", "answer", r#"{"text":"a"}"#),
            ("agent", "pickup", r#"{"by":"w"}"#),
            (
                "agent",
                "note",
                r#"{"body":"  working\non it  ","reopen":true}"#,
            ),
            ("human", "answer", r#"{"text":"b"}"#),
            ("agent", "pickup", "{}"),
            (
                "agent",
                "close",
                r#"{"outcome":"done","links":["https://a","https://b"]}"#,
            ),
        ]);
        assert_eq!(
            thread_texts(&case, &Actors::default()),
            vec![
                vec![],
                strs(&["reply: a"]),
                strs(&["by w"]),
                strs(&["reopened the case", "working", "on it"]),
                strs(&["reply: b"]),
                vec![],
                strs(&["done", "https://a", "https://b"]),
            ]
        );
    }

    #[test]
    fn actors_are_named_unless_they_are_not() {
        let open = |actor: &str| {
            format!(r#"{{"kind":"question","title":"Q","actor":{{"id":"{actor}"}}}}"#)
        };
        let named = fold(&[("agent", "open", &open(AGENT))]);
        assert_eq!(thread_texts(&named, &actors())[0], strs(&["by builder"]));
        let nameless = fold(&[("agent", "open", &open(NAMELESS))]);
        assert!(thread_texts(&nameless, &actors())[0].is_empty());
        let gone = fold(&[("agent", "open", &open(GONE))]);
        assert_eq!(
            thread_texts(&gone, &actors())[0],
            strs(&["by deleted user"])
        );
        let none = fold(&[("agent", "open", QUESTION)]);
        assert!(thread_texts(&none, &actors())[0].is_empty());
    }

    #[test]
    fn a_pickup_by_its_own_actor_has_one_line() {
        let case = |by: &str| {
            fold(&[
                ("agent", "open", QUESTION),
                ("human", "answer", r#"{"text":"a"}"#),
                (
                    "agent",
                    "pickup",
                    &format!(r#"{{"by":"{by}","actor":{{"id":"{AGENT}"}}}}"#),
                ),
            ])
        };
        assert_eq!(
            thread_texts(&case("builder"), &actors())[2],
            strs(&["by builder"])
        );
        assert_eq!(
            thread_texts(&case("other"), &actors())[2],
            strs(&["by builder", "by other"])
        );
    }

    #[test]
    fn a_message_says_who_it_went_to() {
        let to = |open: &str| {
            let case = fold(&[("human", "open", open)]);
            thread_texts(&case, &Actors::default())[0].clone()
        };
        assert_eq!(
            to(r#"{"kind":"message","title":"M","worker":"w1","labels":["x","y"]}"#),
            strs(&["sent to w1 or label x or label y"])
        );
        assert_eq!(
            to(r#"{"kind":"message","title":"M","labels":["x"]}"#),
            strs(&["sent to label x"])
        );
        // Only a message says it; a question with a worker does not.
        let question = fold(&[(
            "agent",
            "open",
            r#"{"kind":"question","title":"Q","worker":"w1"}"#,
        )]);
        assert!(thread_texts(&question, &Actors::default())[0].is_empty());
    }

    #[test]
    fn a_note_or_amend_after_an_answer_reopens_the_case() {
        let case = fold(&[
            ("agent", "open", QUESTION),
            ("human", "answer", r#"{"text":"a"}"#),
            ("agent", "amend", r#"{"labels":["l"],"reopen":true}"#),
            ("human", "answer", r#"{"text":"b"}"#),
            ("agent", "pickup", "{}"),
            ("agent", "amend", r#"{"labels":["m"],"reopen":true}"#),
        ]);
        assert_eq!(
            thread_texts(&case, &Actors::default()),
            vec![
                vec![],
                strs(&["reply: a"]),
                strs(&["reopened the case", "added label: l"]),
                strs(&["reply: b"]),
                vec![],
                strs(&["reopened the case", "added label: m"]),
            ]
        );
        // An amend of an open case does not reopen it.
        let open = fold(&[
            ("agent", "open", QUESTION),
            ("agent", "amend", r#"{"labels":["l"]}"#),
        ]);
        assert_eq!(
            thread_texts(&open, &Actors::default())[1],
            strs(&["added label: l"])
        );
    }

    const APPROVAL: &str = r#"{"kind":"approval","title":"Old","body":"old body","rows":[{"id":"r0","label":"Zero","script":"s","link":"https://l"}],"links":["https://gone"],"context":"old context"}"#;

    #[test]
    fn an_amend_says_each_change() {
        let case = fold(&[
            ("agent", "open", APPROVAL),
            (
                "agent",
                "amend",
                r#"{"title":"New","body":"new body","rows":[{"id":"r1","label":"Row","script":"s","link":"https://l"}],"drop_links":["https://gone"],"links":["https://new"],"labels":["l1"],"context":"new context"}"#,
            ),
        ]);
        let events = thread(&case, &Actors::default());
        let lines = &events[1].1;
        assert_eq!(
            texts(lines),
            [
                "replaced the title: New",
                "replaced the body",
                "added row [r1] Row",
                "dropped link: https://gone",
                "added link: https://new",
                "added label: l1",
                "replaced the context: new context",
            ]
        );
        assert_eq!(lines[1].previous, Some(Previous::Body("old body".into())));
        assert_eq!(
            lines[6].previous,
            Some(Previous::Context("old context".into()))
        );
        assert!(
            lines
                .iter()
                .enumerate()
                .all(|(i, l)| i == 1 || i == 6 || l.previous.is_none())
        );
    }

    #[test]
    fn an_amend_of_a_decision_adds_options_and_recommends() {
        let case = fold(&[
            ("agent", "open", DECISION),
            ("agent", "amend", r#"{"options":["gamma"],"recommend":3}"#),
            ("agent", "amend", r#"{"recommend":1}"#),
        ]);
        assert_eq!(
            thread_texts(&case, &Actors::default()),
            vec![
                vec![],
                strs(&["added option: gamma", "recommended 3. gamma"]),
                strs(&["recommended 1. alpha"]),
            ]
        );
    }

    #[test]
    fn an_amend_says_only_what_differs() {
        let case = fold(&[
            (
                "agent",
                "open",
                r#"{"kind":"decision","title":"T","body":"B","options":["a","b"],"recommend":2,"context":"C"}"#,
            ),
            (
                "agent",
                "amend",
                r#"{"title":"T","body":"B","recommend":2,"context":"C","labels":["l"]}"#,
            ),
        ]);
        assert_eq!(
            thread_texts(&case, &Actors::default())[1],
            strs(&["added label: l"])
        );
    }

    #[test]
    fn an_amend_of_a_briefing_says_only_left_out_items_that_differ() {
        let case = fold(&[
            (
                "agent",
                "open",
                r#"{"kind":"briefing","title":"B","body":"x","left_out":["a","b"]}"#,
            ),
            // The same items: nothing to say of them.
            ("agent", "amend", r#"{"left_out":["a","b"],"labels":["l"]}"#),
            ("agent", "amend", r#"{"left_out":["b","c"]}"#),
        ]);
        assert_eq!(
            thread_texts(&case, &Actors::default()),
            vec![
                vec![],
                strs(&["added label: l"]),
                strs(&["replaced the left-out items: b; c"]),
            ]
        );
    }

    #[test]
    fn a_replaced_text_is_compared_with_the_last_one() {
        let case = fold(&[
            (
                "agent",
                "open",
                r#"{"kind":"question","title":"A","body":"one","context":"c1"}"#,
            ),
            (
                "agent",
                "amend",
                r#"{"title":"B","body":"two","context":"c2"}"#,
            ),
            // Back to the open event's text: it differs from the last, so it is a change.
            (
                "agent",
                "amend",
                r#"{"title":"A","body":"one","context":"c1"}"#,
            ),
        ]);
        let events = thread(&case, &Actors::default());
        assert_eq!(
            texts(&events[2].1),
            [
                "replaced the title: A",
                "replaced the body",
                "replaced the context: c1"
            ]
        );
        assert_eq!(events[2].1[1].previous, Some(Previous::Body("two".into())));
        assert_eq!(
            events[2].1[2].previous,
            Some(Previous::Context("c2".into()))
        );
    }

    #[test]
    fn a_blank_previous_text_is_not_kept() {
        let case = fold(&[
            (
                "agent",
                "open",
                r#"{"kind":"question","title":"T","body":"  \n ","context":"\t"}"#,
            ),
            ("agent", "amend", r#"{"body":"now","context":"ctx"}"#),
        ]);
        let events = thread(&case, &Actors::default());
        assert_eq!(
            texts(&events[1].1),
            ["replaced the body", "replaced the context: ctx"]
        );
        assert!(events[1].1.iter().all(|line| line.previous.is_none()));
    }

    #[test]
    fn add_trims_and_splits_lines() {
        let mut lines = Vec::new();
        add(&mut lines, "\n  a\nb  \n");
        add(&mut lines, "   ");
        assert_eq!(texts(&lines), ["a", "b"]);
        let more = replaced("x\ny", "before", Previous::Body);
        assert_eq!(more[0].previous, None);
        assert_eq!(more[1].previous, Some(Previous::Body("before".into())));
        assert!(replaced("  ", "before", Previous::Body).is_empty());
    }
}
