//! The rules a record keeps once it reads: the open's, the amend's, the
//! answer's and the actor's. A rule it breaks is `invalid`, but for an open's
//! kind or urgency this version does not know, which is `unknown`, and for an
//! open or amend by the side that does not open its kind, which is `author`.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::str::FromStr;

use crate::case::Case;
use crate::error::{Category, Error};
use crate::names::{Author, EventType, Kind, Urgency};
use crate::record::{Actor, Amend, Answer, ApprovalRow, Open};

/// Whether `s` is empty once white space is taken off both ends. White space
/// is what has the Unicode `White_Space` property, which is what `str::trim`
/// takes off: a no-break space is white space, a zero-width no-break space is
/// not.
pub(crate) fn blank(s: &str) -> bool {
    s.trim().is_empty()
}

/// Checks an event's actor, when it has one: its id must be 22 characters of
/// the base64url alphabet. The id is never decoded.
pub(crate) fn check_actor(actor: Option<&Actor>) -> Result<(), Error> {
    match actor {
        Some(actor)
            if actor.id.len() != 22
                || !actor
                    .id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') =>
        {
            Err(Error::invalid(format!(
                "the actor's id {:?} is not 22 base64url characters",
                actor.id
            )))
        }
        _ => Ok(()),
    }
}

/// Checks an open by `author`, and gives its kind and urgency. The kind is
/// checked first, then that `author` opens that kind, then the urgency, then
/// the open's other rules. The recommend is checked with the options, and on
/// a replay as on a write. `write` adds the rules only a write applies
/// (AG-02).
pub(crate) fn check_open(
    open: &Open,
    author: Author,
    write: bool,
) -> Result<(Kind, Urgency), Error> {
    let kind = known::<Kind>(&open.kind, "kind")?;
    check_opener(kind, author, EventType::Open)?;
    let urgency = known::<Urgency>(&open.urgency, "urgency")?;
    check_open_shape(open, kind)?;
    check_open_fields(open, kind)?;
    if write {
        check_open_write(open)?;
    }
    Ok((kind, urgency))
}

/// Checks the shape of an open for its kind: its text, the fields only some
/// kinds take, and what its kind needs.
fn check_open_shape(open: &Open, kind: Kind) -> Result<(), Error> {
    check_open_text(open, kind)?;
    check_kind_fields(kind, &open.options, &open.rows)?;
    check_briefing_fields(kind, open.landed, &open.left_out)?;
    check_open_needs(open, kind)
}

/// Checks an open's options, recommend, rows, `for`, `re` and labels.
fn check_open_fields(open: &Open, kind: Kind) -> Result<(), Error> {
    check_blank("option", &open.options)?;
    check_recommend(kind, open.recommend, open.options.len())?;
    check_rows(&open.rows, &[])?;
    if set_blank(&open.for_) {
        return Err(Error::invalid("for is blank"));
    }
    if set_blank(&open.re) {
        return Err(Error::invalid("re is blank"));
    }
    check_labels(&open.labels, &[])
}

/// Whether `s` is given and is only white space.
fn set_blank(s: &str) -> bool {
    !s.is_empty() && blank(s)
}

/// Checks an open's title, and that a briefing has a body.
fn check_open_text(open: &Open, kind: Kind) -> Result<(), Error> {
    if blank(&open.title) {
        return Err(Error::invalid("the title is blank"));
    }
    if kind == Kind::Briefing && blank(&open.body) {
        return Err(Error::invalid("a briefing needs a body"));
    }
    Ok(())
}

/// Checks that an open has what its kind needs: a decision an option, an
/// approval a row, a message a worker or a label.
fn check_open_needs(open: &Open, kind: Kind) -> Result<(), Error> {
    match kind {
        Kind::Decision if open.options.is_empty() => {
            Err(Error::invalid("a decision needs at least one option"))
        }
        Kind::Approval if open.rows.is_empty() => {
            Err(Error::invalid("an approval needs at least one row"))
        }
        // A worker of only white space names no one, on a replay too, where
        // the write rule that refuses it does not run.
        Kind::Message if blank(&open.worker) && open.labels.is_empty() => {
            Err(Error::invalid("a message goes to a worker or a label"))
        }
        _ => Ok(()),
    }
}

/// The rules only a write of an open applies (AG-02): new options, links
/// neither blank nor given twice, and no worker, brief or context of only
/// white space.
fn check_open_write(open: &Open) -> Result<(), Error> {
    check_new("option", &open.options, &[])?;
    check_blank("link", &open.links)?;
    check_new("link", &open.links, &[])?;
    for (field, value) in [
        ("worker", &open.worker),
        ("brief", &open.brief),
        ("context", &open.context),
    ] {
        if set_blank(value) {
            return Err(Error::invalid(format!("the {field} is blank")));
        }
    }
    Ok(())
}

/// Refuses an open or an amend, `event`, by the side that does not open
/// cases of `kind`, as `author`: the human opens and amends a message, and
/// the agent every other kind ([`Kind::opener`]).
pub(crate) fn check_opener(kind: Kind, author: Author, event: EventType) -> Result<(), Error> {
    let opener = kind.opener();
    if author == opener {
        return Ok(());
    }
    Err(Error::new(
        Category::Author,
        format!("the {author} does not write a {kind}'s {event}: the {opener} does"),
    ))
}

/// Reads an open's kind or urgency. One left out is invalid; one this
/// version does not know is unknown.
fn known<T: FromStr<Err = Error>>(value: &str, field: &str) -> Result<T, Error> {
    if value.is_empty() {
        return Err(Error::invalid(format!("the open has no {field}")));
    }
    value.parse()
}

/// Checks an amend against the case as it stands.
pub(crate) fn check_amend(amend: &Amend, case: &Case) -> Result<(), Error> {
    check_amend_fields(amend, case)?;
    check_amend_against(amend, case)?;
    // What changes the question is listed once, in changes_question; the
    // rest are changes that leave it as it was.
    if !changes_question(amend, case) && !changes_lists(amend) && !changes_rest(amend, case) {
        // This also refuses an amend that sets nothing at all, and one that
        // only reopens the case: a reopen asks again, so it says what for.
        return Err(Error::invalid(
            "the amend changes nothing: it adds no options, rows, links or labels, \
             drops no links, and sets no title, body, context, recommend, landed or \
             left out the case does not have",
        ));
    }
    Ok(())
}

/// Checks an amend's fields for the case's kind, and that none it gives is
/// blank; its rows are checked against the case's.
fn check_amend_fields(amend: &Amend, case: &Case) -> Result<(), Error> {
    check_kind_fields(case.kind(), &amend.options, &amend.rows)?;
    check_briefing_fields(case.kind(), amend.landed.is_some(), &amend.left_out)?;
    check_amend_text(amend)?;
    check_blank("option", &amend.options)?;
    check_rows(&amend.rows, case.rows())?;
    check_blank("link", &amend.links)
}

/// Checks an amend's options, recommend, links and labels against the
/// case's: what it adds is new, and what it drops the case has.
fn check_amend_against(amend: &Amend, case: &Case) -> Result<(), Error> {
    check_new("option", &amend.options, case.options())?;
    // The options as amended: a recommend may name one this amend adds.
    check_recommend(
        case.kind(),
        amend.recommend,
        case.options().len() + amend.options.len(),
    )?;
    check_new("link", &amend.links, case.links())?;
    check_dropped_links(amend, case.links())?;
    check_labels(&amend.labels, case.labels())
}

/// Refuses an amend's title, body or context of only white space.
fn check_amend_text(amend: &Amend) -> Result<(), Error> {
    for (field, value) in [
        ("title", &amend.title),
        ("body", &amend.body),
        ("context", &amend.context),
    ] {
        if set_blank(value) {
            return Err(Error::invalid(format!("the {field} is blank")));
        }
    }
    Ok(())
}

/// Whether an amend adds links or labels, or drops links.
fn changes_lists(amend: &Amend) -> bool {
    !amend.links.is_empty() || !amend.drop_links.is_empty() || !amend.labels.is_empty()
}

/// Whether an amend sets a title, context, recommend or `landed` other than
/// the case's: changes that leave the question as it was.
fn changes_rest(amend: &Amend, case: &Case) -> bool {
    (!amend.title.is_empty() && amend.title != case.title())
        || (!amend.context.is_empty() && amend.context != case.context())
        || (amend.recommend != 0 && amend.recommend != case.recommend())
        || amend.landed.is_some_and(|landed| landed != case.landed())
}

/// Refuses a link to drop that the case does not have, one given twice, and
/// one the same amend adds.
fn check_dropped_links(amend: &Amend, have: &[String]) -> Result<(), Error> {
    for (i, link) in amend.drop_links.iter().enumerate() {
        if amend.drop_links[..i].contains(link) {
            return Err(Error::invalid(format!(
                "the link {link:?} is dropped twice"
            )));
        }
        if !have.contains(link) {
            return Err(Error::invalid(format!(
                "the case has no link {link:?} to drop"
            )));
        }
        if amend.links.contains(link) {
            return Err(Error::invalid(format!(
                "the link {link:?} is both added and dropped"
            )));
        }
    }
    Ok(())
}

/// Whether an amend changes the question, and so moves the case's
/// `amend_seq`: it adds options or rows, or sets a body or a briefing's
/// left-out list other than the case's, since an answer files left-out
/// items by number. A title, links, labels, context, the recommend and
/// `landed` do not (ST-04). The
/// recommend is advice about the question, and a build from before it skips
/// an amend that sets only a recommend as changing nothing: were it to move
/// `amend_seq`, the two builds would not agree on one for the same rows. The
/// title is held to the same rule for the same reason.
pub(crate) fn changes_question(amend: &Amend, case: &Case) -> bool {
    !amend.options.is_empty()
        || !amend.rows.is_empty()
        || (!amend.body.is_empty() && amend.body != case.body())
        || (!amend.left_out.is_empty() && amend.left_out != case.left_out())
}

/// Checks an answer against the case as it stands, amended. Its `amend_seq`
/// is not below the case's: that is checked before the state.
pub(crate) fn check_answer(answer: &Answer, case: &Case) -> Result<(), Error> {
    let kind = case.kind();
    let given = check_responses(answer, kind)?;
    if !answer.file.is_empty() && (kind != Kind::Briefing || answer.drop) {
        return Err(Error::invalid(
            "file goes only with a briefing's verdict, not with drop or on another kind",
        ));
    }
    if i128::from(answer.amend_seq) > i128::from(case.amend_seq()) {
        return Err(Error::invalid(format!(
            "the answer saw amend {}, and the case's latest is {}",
            answer.amend_seq,
            case.amend_seq()
        )));
    }
    if answer.drop {
        if given > 1 {
            return Err(Error::invalid(
                "an answer drops the case or answers it, not both",
            ));
        }
        return Ok(());
    }
    match kind {
        Kind::Decision => check_choice(answer, given, case.options().len()),
        Kind::Approval => check_verdicts(answer, case.rows()),
        Kind::Signoff => check_signoff(answer),
        Kind::Briefing => check_briefing(answer, case.left_out()),
        Kind::Stuck | Kind::Question => {
            if blank(&answer.text) {
                return Err(Error::invalid("the text is blank"));
            }
            Ok(())
        }
        Kind::Fyi | Kind::Message => Ok(()),
    }
}

/// The responses an answer to a `kind` takes.
fn takes(kind: Kind) -> &'static [&'static str] {
    match kind {
        Kind::Decision => &["choice", "other", "drop"],
        Kind::Approval => &["rows", "drop"],
        Kind::Signoff => &["signoff", "drop"],
        Kind::Briefing => &["briefing", "drop"],
        Kind::Stuck | Kind::Question => &["text", "drop"],
        Kind::Fyi => &["ack", "drop"],
        // A message takes no answer. It is never open, so its state refuses
        // one first.
        Kind::Message => &[],
    }
}

/// Refuses a response a `kind` answer does not take, then an answer with
/// none, and gives how many it has.
fn check_responses(answer: &Answer, kind: Kind) -> Result<usize, Error> {
    let responses = [
        ("choice", answer.choice != 0),
        ("other", answer.other),
        ("rows", !answer.rows.is_empty()),
        ("signoff", !answer.signoff.is_empty()),
        ("briefing", !answer.briefing.is_empty()),
        ("text", !answer.text.is_empty()),
        ("drop", answer.drop),
        ("ack", answer.ack),
    ];
    let takes = takes(kind);
    let given: Vec<&str> = responses
        .iter()
        .filter(|(_, given)| *given)
        .map(|(field, _)| *field)
        .collect();
    if let Some(field) = given.iter().find(|field| !takes.contains(field)) {
        return Err(Error::invalid(format!(
            "a {kind} answer does not take {field}"
        )));
    }
    if given.is_empty() {
        return Err(Error::invalid(format!("the answer has no {kind} response")));
    }
    Ok(given.len())
}

/// A decision's answer chooses one of its `options` options, or other with a
/// note.
fn check_choice(answer: &Answer, given: usize, options: usize) -> Result<(), Error> {
    if given > 1 {
        return Err(Error::invalid(
            "an answer chooses an option or other, not both",
        ));
    }
    if answer.other {
        if blank(&answer.note) {
            return Err(Error::invalid("other needs a note"));
        }
    } else if !names_option(answer.choice, options) {
        return Err(Error::invalid(format!(
            "choice {} is not one of the {options} options",
            answer.choice
        )));
    }
    Ok(())
}

/// An approval's answer gives each of the case's `rows` one verdict.
fn check_verdicts(answer: &Answer, rows: &[ApprovalRow]) -> Result<(), Error> {
    for (i, verdict) in answer.rows.iter().enumerate() {
        if !rows.iter().any(|row| row.id == verdict.id) {
            return Err(Error::invalid(format!(
                "the case has no row {:?}",
                verdict.id
            )));
        }
        if answer.rows[..i].iter().any(|v| v.id == verdict.id) {
            return Err(Error::invalid(format!(
                "row {:?} has two verdicts",
                verdict.id
            )));
        }
        if !matches!(verdict.verdict.as_str(), "approve" | "hold" | "reject") {
            return Err(Error::invalid(format!(
                "{:?} is not approve, hold or reject",
                verdict.verdict
            )));
        }
    }
    if let Some(row) = rows
        .iter()
        .find(|row| !answer.rows.iter().any(|v| v.id == row.id))
    {
        return Err(Error::invalid(format!("row {:?} has no verdict", row.id)));
    }
    Ok(())
}

/// A signoff's answer accepts, or asks for changes with a note.
fn check_signoff(answer: &Answer) -> Result<(), Error> {
    match answer.signoff.as_str() {
        "changes" if blank(&answer.note) => Err(Error::invalid("asking for changes needs a note")),
        "accept" | "changes" => Ok(()),
        other => Err(Error::invalid(format!(
            "{other:?} is not accept or changes"
        ))),
    }
}

/// A briefing's answer accepts, or sends it back with a note, and files any
/// of its `left_out` items: each by number from 1, and none twice.
fn check_briefing(answer: &Answer, left_out: &[String]) -> Result<(), Error> {
    match answer.briefing.as_str() {
        "send_back" if blank(&answer.note) => {
            return Err(Error::invalid("sending back needs a reason"));
        }
        "accept" | "send_back" => {}
        other => {
            return Err(Error::invalid(format!(
                "{other:?} is not accept or send_back"
            )));
        }
    }
    for (i, n) in answer.file.iter().enumerate() {
        if !names_option(*n, left_out.len()) {
            return Err(Error::invalid(format!(
                "file {n} is not one of the {} left-out items",
                left_out.len()
            )));
        }
        if answer.file[..i].contains(n) {
            return Err(Error::invalid(format!("item {n} is filed twice")));
        }
    }
    Ok(())
}

/// The most items a briefing leaves out, and the most characters in one.
const LEFT_OUT_ITEMS: usize = 10;
const LEFT_OUT_CHARS: usize = 300;

/// `landed` and `left_out` go only on a briefing. Its left-out items are at
/// most 10, each at most 300 characters, none blank and none given twice.
fn check_briefing_fields(kind: Kind, landed: bool, left_out: &[String]) -> Result<(), Error> {
    if kind != Kind::Briefing {
        if landed {
            return Err(Error::invalid(format!("a {kind} has no landed")));
        }
        if !left_out.is_empty() {
            return Err(Error::invalid(format!("a {kind} has no left_out")));
        }
        return Ok(());
    }
    if left_out.len() > LEFT_OUT_ITEMS {
        return Err(Error::invalid(format!(
            "a briefing leaves out at most {LEFT_OUT_ITEMS} items, not {}",
            left_out.len()
        )));
    }
    check_blank("left-out item", left_out)?;
    if let Some(i) = left_out
        .iter()
        .position(|item| item.chars().count() > LEFT_OUT_CHARS)
    {
        return Err(Error::invalid(format!(
            "left-out item {} is over {LEFT_OUT_CHARS} characters",
            i + 1
        )));
    }
    check_new("left-out item", left_out, &[])
}

/// Options go only on a decision, and rows only on an approval.
fn check_kind_fields(kind: Kind, options: &[String], rows: &[ApprovalRow]) -> Result<(), Error> {
    if kind != Kind::Decision && !options.is_empty() {
        return Err(Error::invalid(format!("a {kind} has no options")));
    }
    if kind != Kind::Approval && !rows.is_empty() {
        return Err(Error::invalid(format!("a {kind} has no rows")));
    }
    Ok(())
}

/// A recommend goes only on a decision, and names one of its `options`
/// options, numbered from 1 as a choice is. The number after the last is
/// other, which is not an option. 0 is a recommend left out.
fn check_recommend(kind: Kind, recommend: i64, options: usize) -> Result<(), Error> {
    if recommend == 0 {
        return Ok(());
    }
    if kind != Kind::Decision {
        return Err(Error::invalid(format!(
            "only a decision takes a recommend, and the case's kind is {kind}"
        )));
    }
    if !names_option(recommend, options) {
        return Err(Error::invalid(format!(
            "recommend {recommend} is not one of the {options} options"
        )));
    }
    Ok(())
}

/// Whether `n` names one of `options` options, numbered from 1, as a choice
/// or a recommend names one. The number after the last is other, which is
/// not an option.
fn names_option(n: i64, options: usize) -> bool {
    usize::try_from(n).is_ok_and(|n| (1..=options).contains(&n))
}

/// Refuses a blank value in `values`, such as an option or a link.
fn check_blank(what: &str, values: &[String]) -> Result<(), Error> {
    match values.iter().position(|value| blank(value)) {
        Some(i) => Err(Error::invalid(format!("{what} {} is blank", i + 1))),
        None => Ok(()),
    }
}

fn check_labels(added: &[String], have: &[String]) -> Result<(), Error> {
    check_blank("label", added)?;
    check_new("label", added, have)
}

/// Refuses a value given twice in `added`, or one the case already has.
fn check_new(what: &str, added: &[String], have: &[String]) -> Result<(), Error> {
    for (i, value) in added.iter().enumerate() {
        if added[..i].contains(value) {
            return Err(Error::invalid(format!(
                "the {what} {value:?} is given twice"
            )));
        }
        if have.contains(value) {
            return Err(Error::invalid(format!(
                "the case has the {what} {value:?} already"
            )));
        }
    }
    Ok(())
}

/// Checks rows to add to a case that has `have`. A row's id is letters,
/// digits, `.`, `-` and `_`, unique in the case, and its label, script and
/// link are not blank. Its `by` is left out, for the agent, or `human`.
fn check_rows(rows: &[ApprovalRow], have: &[ApprovalRow]) -> Result<(), Error> {
    for (i, row) in rows.iter().enumerate() {
        let id = &row.id;
        let id_fits = !id.is_empty()
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'));
        if !id_fits {
            return Err(Error::invalid(format!(
                "row {}: the id {id:?} is not letters, digits, '.', '-' and '_'",
                i + 1
            )));
        }
        if rows[..i].iter().chain(have).any(|other| &other.id == id) {
            return Err(Error::invalid(format!(
                "the row id {id:?} is not unique in the case"
            )));
        }
        for (field, value) in [
            ("label", &row.label),
            ("script", &row.script),
            ("link", &row.link),
        ] {
            if blank(value) {
                return Err(Error::invalid(format!("row {id:?} has a blank {field}")));
            }
        }
        if !matches!(row.by.as_str(), "" | "human") {
            return Err(Error::invalid(format!(
                "row {id:?}: by is {:?}, not human or left out",
                row.by
            )));
        }
    }
    Ok(())
}
