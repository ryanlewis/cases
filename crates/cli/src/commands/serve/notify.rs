//! Desktop notifications (IN-08): which cases newly landed on the human, and
//! the feed of them that an inbox tab reads.
//!
//! The engine is shown the store's cases after each read that finds the
//! store moved. It compares their events, by case and seq, with the ones it
//! has seen, so an event counts as new when it appears in the store, not by
//! its time. The first read only records what is there. A case that leaves
//! the store is forgotten, so what the engine keeps is bounded by the live
//! store.

use std::collections::{HashMap, HashSet, VecDeque};

use cases_core::{Author, Case, Event, EventType, Kind, State, Time};
use cases_store_sqlite::{Cases, Key};
use serde::Serialize;

use crate::output;

/// How many notifications the feed keeps.
pub(crate) const FEED_SIZE: usize = 50;

/// What put a case on the human.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Name {
    /// The agent opened it.
    Open,
    /// The agent followed up on an answer with a note or an amend that
    /// reopened it.
    Reopen,
    /// The agent's first note after the human resumed it: the case was open
    /// already, but the human is waiting on it.
    Reply,
}

impl Name {
    fn as_str(self) -> &'static str {
        match self {
            Name::Open => "open",
            Name::Reopen => "reopen",
            Name::Reply => "reply",
        }
    }
}

/// A case that newly landed on the human: its key, and the seq of the event
/// that put it there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Landed {
    pub(crate) key: Key,
    pub(crate) seq: u64,
    pub(crate) name: Name,
}

/// A case as the engine tells it from the others: its key, and when it was
/// opened. Keys start again at 1 in a store made again at the path, and the
/// open time tells its cases from the old store's.
type CaseId = (Key, Option<Time>);

/// Decides, read by read, which cases newly landed on the human.
#[derive(Debug, Default)]
pub(crate) struct Engine {
    /// For each case in the last read, the seqs of the events looked at
    /// already; `None` until the first read.
    seen: Option<HashMap<CaseId, HashSet<u64>>>,
}

impl Engine {
    /// Takes the cases one read of the store gave, and gives the cases that
    /// landed on the human since the read before. The first call only
    /// records what is there.
    pub(crate) fn observe(&mut self, cases: &Cases) -> Vec<Landed> {
        let first = self.seen.is_none();
        let mut before = self.seen.take().unwrap_or_default();
        let mut seen = HashMap::with_capacity(cases.len());
        let mut landed = Vec::new();
        for (key, case) in cases {
            let Ok(case) = case else { continue };
            let id = (*key, case.opened_at());
            let mut seqs = before.remove(&id).unwrap_or_default();
            let fresh: HashSet<u64> = case
                .events()
                .iter()
                .map(|event| event.seq)
                .filter(|seq| seqs.insert(*seq))
                .collect();
            seen.insert(id, seqs);
            if first || fresh.is_empty() {
                continue;
            }
            if let Some((event, name)) = on_the_human(case)
                && fresh.contains(&event.seq)
            {
                landed.push(Landed {
                    key: *key,
                    seq: event.seq,
                    name,
                });
            }
        }
        self.seen = Some(seen);
        landed
    }
}

/// Whether the case is on the human because of an event that notifies, and
/// which event: the case is open, and the latest event that put it there,
/// looking past amends and notes on the open case, is the agent opening it
/// or reopening it with a note or an amend. When that event is the human
/// resuming it,
/// the agent's first note after the resume is the one that put it on the
/// human. The agent never resumes a case (HU-05). A briefing never
/// notifies: it waits in a tab of its own (IN-14).
pub(crate) fn on_the_human(case: &Case) -> Option<(&Event, Name)> {
    let events = case.events();
    if case.state() != State::Open || case.kind() == Kind::Briefing || events.is_empty() {
        return None;
    }
    let mut at = events.len() - 1;
    let mut note = None;
    while at > 0 {
        let event = &events[at];
        let on_open = event.from == Some(State::Open);
        match event.event() {
            EventType::Note if on_open => note = Some(at),
            EventType::Amend if on_open => {}
            _ => break,
        }
        at -= 1;
    }
    let event = &events[at];
    match event.event() {
        EventType::Resume if event.author == Author::Human => {
            note.map(|note| (&events[note], Name::Reply))
        }
        EventType::Open => Some((event, Name::Open)),
        // A note or an amend not on the open case reopened it.
        EventType::Note | EventType::Amend => Some((event, Name::Reopen)),
        _ => None,
    }
}

/// One notification. `title`, `body` and `url` are what a tab shows and
/// where a click goes; `tag` is the same in every tab that shows it, so the
/// desktop shows it once.
#[derive(Debug, Serialize)]
pub(crate) struct Item {
    id: u64,
    sink: &'static str,
    tag: String,
    title: String,
    body: String,
    url: String,
    queued_at: Time,
    event: EventRef,
    /// The case as `show --json` gave it when the item was queued.
    case: serde_json::Value,
}

/// The event a notification is about.
#[derive(Debug, Serialize)]
struct EventRef {
    name: &'static str,
    seq: u64,
    author: &'static str,
    event: &'static str,
    file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    at: Option<Time>,
}

impl Item {
    /// The notification that `event` put the case `key` on the human, queued
    /// at `now`, with `json`, the case as `show --json` gives it.
    pub(crate) fn new(
        key: Key,
        case: &Case,
        event: &Event,
        name: Name,
        now: Time,
        json: serde_json::Value,
    ) -> Item {
        let id = key.to_string();
        let mut body = format!("{} {}", case.urgency().as_str(), case.kind().as_str());
        match name {
            Name::Open if !case.worker().is_empty() => {
                body.push_str(" · from ");
                body.push_str(case.worker());
            }
            Name::Open => {}
            Name::Reopen => body.push_str(" · the agent followed up"),
            Name::Reply => body.push_str(" · the agent replied"),
        }
        Item {
            id: 0,
            sink: "browser",
            tag: format!("{id}/{}", event.file()),
            title: case.title().to_owned(),
            body,
            url: format!("/cases/{}", output::titled(key, case.title())),
            queued_at: now,
            event: EventRef {
                name: name.as_str(),
                seq: event.seq,
                author: event.author.as_str(),
                event: event.event().as_str(),
                file: event.file(),
                at: event.at(),
            },
            case: json,
        }
    }
}

/// The last [`FEED_SIZE`] notifications, numbered from 1, and the boot id,
/// which is new each time serve starts, so that a tab knows to drop the
/// number it kept.
#[derive(Debug)]
pub(crate) struct Feed {
    boot: String,
    latest: u64,
    items: VecDeque<Item>,
}

/// What a tab gets from the feed: the items after the number it asked for,
/// the latest number, and the boot id.
#[derive(Debug, Serialize)]
pub(crate) struct Page<'a> {
    boot: &'a str,
    latest: u64,
    items: Vec<&'a Item>,
}

impl Feed {
    pub(crate) fn new(boot: String) -> Feed {
        Feed {
            boot,
            latest: 0,
            items: VecDeque::new(),
        }
    }

    /// Adds `item` as the next number, and forgets the oldest past
    /// [`FEED_SIZE`].
    pub(crate) fn add(&mut self, mut item: Item) {
        self.latest += 1;
        item.id = self.latest;
        self.items.push_back(item);
        while self.items.len() > FEED_SIZE {
            self.items.pop_front();
        }
    }

    /// How many notifications the feed has taken: the latest number.
    pub(crate) fn latest(&self) -> u64 {
        self.latest
    }

    /// The items numbered above `after`, oldest first.
    pub(crate) fn after(&self, after: u64) -> Page<'_> {
        Page {
            boot: &self.boot,
            latest: self.latest,
            items: self.items.iter().filter(|item| item.id > after).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use cases_core::{Append, Store};
    use cases_store_sqlite::{Db, Writer};

    use super::*;

    fn now() -> Time {
        "2026-09-17T09:00:00Z".parse().unwrap()
    }

    /// A store of its own, not made yet.
    fn scratch() -> (tempfile::TempDir, Db) {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::new(dir.path().join("cases.db"));
        (dir, db)
    }

    fn append(author: &str, event: &str, data: &str) -> Append {
        Append {
            author: author.into(),
            event: event.into(),
            data: data.into(),
        }
    }

    /// Opens a case of `kind` and `urgency` titled `title`, from the worker
    /// bun-pins.
    fn open(db: &Db, kind: &str, urgency: &str, title: &str) -> Key {
        let options = if kind == "decision" {
            r#","options":["Pin","Float"]"#
        } else {
            ""
        };
        let data = format!(
            r#"{{"kind":"{kind}","urgency":"{urgency}","title":"{title}","worker":"bun-pins"{options}}}"#
        );
        db.create(&append("agent", "open", &data), &Writer::new(None))
            .unwrap()
            .0
    }

    fn write(db: &Db, key: Key, author: &str, event: &str, data: &str) {
        db.append(&key, &append(author, event, data), &Writer::new(None), None)
            .unwrap();
    }

    /// The cases in the store, as serve's poller reads them.
    fn read(db: &Db) -> Cases {
        match db.poller().poll() {
            Ok(cases) => cases.clone(),
            Err(cases_store_sqlite::Error::Missing { .. }) => Cases::new(),
            Err(error) => panic!("{error}"),
        }
    }

    fn titles(cases: &Cases, landed: &[Landed]) -> Vec<String> {
        landed
            .iter()
            .map(|landed| {
                let case = cases[&landed.key].as_ref().unwrap();
                format!("{}:{}", case.title(), landed.name.as_str())
            })
            .collect()
    }

    #[test]
    fn the_first_read_only_records_the_store() {
        let (_dir, db) = scratch();
        open(&db, "fyi", "blocking", "Already here");
        let mut engine = Engine::default();
        assert!(engine.observe(&read(&db)).is_empty());
        assert!(engine.observe(&read(&db)).is_empty());
    }

    #[test]
    fn an_empty_first_read_still_counts() {
        let (_dir, db) = scratch();
        let mut engine = Engine::default();
        engine.observe(&read(&db));
        open(&db, "fyi", "today", "First");
        assert_eq!(engine.observe(&read(&db)).len(), 1);
    }

    #[test]
    fn what_lands_on_the_human() {
        let (_dir, db) = scratch();
        let answered = open(&db, "decision", "today", "Answered");
        let picked_up = open(&db, "decision", "today", "Picked up");
        let amended = open(&db, "signoff", "today", "Amended");
        let resumed = open(&db, "stuck", "whenever", "Human resumes");
        let noted = open(&db, "question", "today", "Noted while open");
        let withdrawn = open(&db, "fyi", "today", "Withdrawn");
        write(&db, answered, "human", "answer", r#"{"choice":1}"#);
        write(&db, picked_up, "human", "answer", r#"{"choice":1}"#);
        write(&db, picked_up, "agent", "pickup", "{}");
        write(
            &db,
            amended,
            "human",
            "answer",
            r#"{"signoff":"changes","note":"Fix it."}"#,
        );
        write(&db, amended, "agent", "pickup", "{}");
        write(&db, resumed, "human", "park", "{}");

        let mut engine = Engine::default();
        engine.observe(&read(&db));
        let note = r#"{"body":"One more thing?"}"#;
        let reopen = r#"{"body":"One more thing?","reopen":true}"#;
        write(&db, answered, "agent", "note", reopen);
        write(&db, picked_up, "agent", "note", reopen);
        let amend = r#"{"body":"Fixed.","reopen":true}"#;
        write(&db, amended, "agent", "amend", amend);
        write(&db, resumed, "human", "resume", "{}");
        write(&db, noted, "agent", "note", note);
        write(&db, noted, "agent", "amend", r#"{"context":"More."}"#);
        write(&db, withdrawn, "agent", "withdraw", "{}");
        open(&db, "fyi", "whenever", "New");
        let cases = read(&db);
        assert_eq!(
            titles(&cases, &engine.observe(&cases)),
            [
                "Answered:reopen",
                "Picked up:reopen",
                "Amended:reopen",
                "New:open"
            ]
        );

        // The agent's reply to the human's resume lands the case on the
        // human; an amend before it does not, and its next note does not
        // fire again.
        write(&db, resumed, "agent", "amend", r#"{"context":"Looking."}"#);
        write(&db, resumed, "agent", "note", note);
        let cases = read(&db);
        let landed = engine.observe(&cases);
        assert_eq!(titles(&cases, &landed), ["Human resumes:reply"]);
        assert_eq!(landed[0].seq, 5);
        write(&db, resumed, "agent", "note", note);
        assert!(engine.observe(&read(&db)).is_empty());
    }

    /// A message waits on an agent, never on the human: sent, edited, taken
    /// or closed, it lands nowhere (IN-08, IN-13).
    #[test]
    fn a_message_never_lands_on_the_human() {
        let (_dir, db) = scratch();
        let mut engine = Engine::default();
        engine.observe(&read(&db));
        let data =
            r#"{"kind":"message","urgency":"blocking","title":"Rebase","worker":"bun-pins"}"#;
        let key = db
            .create(&append("human", "open", data), &Writer::new(None))
            .unwrap()
            .0;
        assert!(engine.observe(&read(&db)).is_empty());
        write(&db, key, "human", "amend", r#"{"body":"Then test."}"#);
        assert!(engine.observe(&read(&db)).is_empty());
        write(&db, key, "agent", "pickup", r#"{"by":"bun-pins"}"#);
        assert!(engine.observe(&read(&db)).is_empty());
        write(&db, key, "agent", "close", r#"{"outcome":"Done."}"#);
        assert!(engine.observe(&read(&db)).is_empty());
    }

    /// A briefing waits in its own tab: opened, or reopened by an amend
    /// after a send back, it notifies nothing (IN-14).
    #[test]
    fn a_briefing_never_lands_on_the_human() {
        let (_dir, db) = scratch();
        let mut engine = Engine::default();
        engine.observe(&read(&db));
        let data = r#"{"kind":"briefing","urgency":"blocking","title":"Parser split","body":"What changed.","worker":"bun-pins"}"#;
        let key = db
            .create(&append("agent", "open", data), &Writer::new(None))
            .unwrap()
            .0;
        assert!(engine.observe(&read(&db)).is_empty());
        write(
            &db,
            key,
            "human",
            "answer",
            r#"{"briefing":"send_back","note":"Split it.","amend_seq":0}"#,
        );
        write(&db, key, "agent", "pickup", r#"{"by":"bun-pins"}"#);
        assert!(engine.observe(&read(&db)).is_empty());
        write(
            &db,
            key,
            "agent",
            "amend",
            r#"{"body":"Split.","reopen":true}"#,
        );
        assert!(engine.observe(&read(&db)).is_empty());
        // A question opened beside it still lands.
        let question = open(&db, "question", "blocking", "Quick");
        let landed = engine.observe(&read(&db));
        assert_eq!(landed.len(), 1);
        assert_eq!(landed[0].key, question);
    }

    #[test]
    fn a_case_opened_and_followed_up_between_reads_lands_once_by_its_open() {
        let (_dir, db) = scratch();
        let mut engine = Engine::default();
        engine.observe(&read(&db));
        let key = open(&db, "question", "blocking", "Quick");
        write(
            &db,
            key,
            "agent",
            "amend",
            r#"{"context":"Seen on the mirror."}"#,
        );
        write(&db, key, "agent", "note", r#"{"body":"Still there?"}"#);
        let landed = engine.observe(&read(&db));
        assert_eq!(
            landed,
            [Landed {
                key,
                seq: 1,
                name: Name::Open
            }]
        );
    }

    #[test]
    fn a_case_answered_before_it_was_seen_does_not_land() {
        let (_dir, db) = scratch();
        let mut engine = Engine::default();
        engine.observe(&read(&db));
        let key = open(&db, "fyi", "blocking", "Gone by");
        write(&db, key, "human", "answer", r#"{"ack":true}"#);
        assert!(engine.observe(&read(&db)).is_empty());
    }

    #[test]
    fn a_case_withdrawn_before_it_was_seen_does_not_land() {
        let (_dir, db) = scratch();
        let mut engine = Engine::default();
        engine.observe(&read(&db));
        let key = open(&db, "fyi", "blocking", "Gone by");
        write(&db, key, "agent", "withdraw", "{}");
        assert!(engine.observe(&read(&db)).is_empty());
    }

    #[test]
    fn a_case_that_leaves_the_store_is_forgotten() {
        let (_dir, db) = scratch();
        let mut engine = Engine::default();
        let kept = open(&db, "fyi", "today", "Stays");
        engine.observe(&read(&db));
        for round in 0..20 {
            let key = open(&db, "fyi", "blocking", "Passing");
            assert_eq!(engine.observe(&read(&db)).len(), 1, "round {round}");
            write(&db, key, "human", "answer", r#"{"ack":true}"#);
            write(&db, key, "agent", "pickup", "{}");
            write(&db, key, "agent", "close", r#"{"outcome":"Done."}"#);
            engine.observe(&read(&db));
            db.archive(&key, None).unwrap();
            assert!(engine.observe(&read(&db)).is_empty(), "round {round}");
            let seen = engine.seen.as_ref().unwrap();
            let keys: Vec<_> = seen.keys().map(|(key, _)| key).collect();
            assert_eq!(keys, [&kept], "round {round}");
        }
    }

    /// A store thrown away and made again gives its first case the key the
    /// old store's first case had. It is still a new case, and lands.
    #[test]
    fn a_case_of_a_store_made_again_lands_though_its_key_was_seen() {
        let store = |at: &str| {
            let (dir, db) = scratch();
            let data =
                format!(r#"{{"kind":"fyi","urgency":"blocking","title":"T","opened_at":"{at}"}}"#);
            let (key, _) = db
                .create(&append("agent", "open", &data), &Writer::new(None))
                .unwrap();
            (dir, db, key)
        };
        let (_old_dir, old, old_key) = store("2026-09-17T08:00:00Z");
        let mut engine = Engine::default();
        engine.observe(&read(&old));
        let (_new_dir, new, key) = store("2026-09-17T09:00:00Z");
        assert_eq!(key, old_key, "keys start again at 1 in a new store");
        assert_eq!(
            engine.observe(&read(&new)),
            [Landed {
                key,
                seq: 1,
                name: Name::Open
            }]
        );
    }

    /// A copy of the store put in its place has the same keys and open
    /// times, so its cases are the ones seen. What the copy holds that the
    /// old store did not lands; an event at a seq seen already does not,
    /// whatever it is; and an untouched copy lands nothing.
    #[test]
    fn a_copy_of_the_store_put_in_its_place_lands_only_what_is_new() {
        // A store whose decision was answered and picked up: on no one.
        let store = || {
            let (dir, db) = scratch();
            let data = r#"{"kind":"decision","urgency":"today","title":"Copied","options":["Pin","Float"],"opened_at":"2026-09-17T08:00:00Z"}"#;
            let (key, _) = db
                .create(&append("agent", "open", data), &Writer::new(None))
                .unwrap();
            write(&db, key, "human", "answer", r#"{"choice":1}"#);
            write(&db, key, "agent", "pickup", "{}");
            (dir, db, key)
        };
        let reopen = r#"{"body":"One more thing?","reopen":true}"#;
        let (_old_dir, old, key) = store();
        let mut engine = Engine::default();
        engine.observe(&read(&old));
        let (_same_dir, same, _) = store();
        assert!(engine.observe(&read(&same)).is_empty());
        let (_more_dir, more, _) = store();
        write(&more, key, "agent", "note", reopen);
        assert_eq!(
            engine.observe(&read(&more)),
            [Landed {
                key,
                seq: 4,
                name: Name::Reopen
            }]
        );
        // Another fourth event, in a copy without the note: its seq was
        // seen, so it is not new.
        let (_other_dir, other, _) = store();
        write(
            &other,
            key,
            "agent",
            "amend",
            r#"{"body":"Fixed.","reopen":true}"#,
        );
        assert!(engine.observe(&read(&other)).is_empty());
        // The old store back, and the note in it later: seen already.
        assert!(engine.observe(&read(&old)).is_empty());
        write(&old, key, "agent", "note", reopen);
        assert!(engine.observe(&read(&old)).is_empty());
    }

    /// Several cases moving in one read land each by its own event, beside
    /// cases that did not move.
    #[test]
    fn several_cases_moving_in_one_read_land_each_once() {
        let (_dir, db) = scratch();
        let still = open(&db, "fyi", "today", "Still");
        let answered = open(&db, "decision", "today", "Answered");
        write(&db, answered, "human", "answer", r#"{"choice":1}"#);
        write(&db, answered, "agent", "pickup", "{}");
        let mut engine = Engine::default();
        engine.observe(&read(&db));
        write(
            &db,
            answered,
            "agent",
            "note",
            r#"{"body":"Again?","reopen":true}"#,
        );
        let first = open(&db, "question", "blocking", "First");
        let second = open(&db, "fyi", "whenever", "Second");
        let landed = engine.observe(&read(&db));
        assert_eq!(
            landed,
            [
                Landed {
                    key: answered,
                    seq: 4,
                    name: Name::Reopen
                },
                Landed {
                    key: first,
                    seq: 1,
                    name: Name::Open
                },
                Landed {
                    key: second,
                    seq: 1,
                    name: Name::Open
                },
            ]
        );
        assert!(!landed.iter().any(|landed| landed.key == still));
        assert!(engine.observe(&read(&db)).is_empty());
    }

    #[test]
    fn an_item_says_what_landed_and_where_it_is() {
        let (_dir, db) = scratch();
        let key = open(&db, "decision", "blocking", "Pin bun?");
        let cases = read(&db);
        let case = cases[&key].as_ref().unwrap();
        let (event, name) = on_the_human(case).unwrap();
        let mut feed = Feed::new("boot".into());
        feed.add(Item::new(
            key,
            case,
            event,
            name,
            now(),
            serde_json::json!({"id": key.to_string()}),
        ));
        let page = serde_json::to_value(feed.after(0)).unwrap();
        let id = key.to_string();
        assert_eq!(
            page,
            serde_json::json!({
                "boot": "boot",
                "latest": 1,
                "items": [{
                    "id": 1,
                    "sink": "browser",
                    "tag": format!("{id}/0001-agent-open.json"),
                    "title": "Pin bun?",
                    "body": "blocking decision · from bun-pins",
                    "url": format!("/cases/{id}-pin-bun"),
                    "queued_at": "2026-09-17T09:00:00Z",
                    "event": {
                        "name": "open",
                        "seq": 1,
                        "author": "agent",
                        "event": "open",
                        "file": "0001-agent-open.json",
                        "at": case.opened_at().unwrap().to_string(),
                    },
                    "case": {"id": id},
                }],
            })
        );
    }

    #[test]
    fn the_feed_keeps_the_last_items_and_counts_on() {
        let (_dir, db) = scratch();
        let key = open(&db, "fyi", "today", "Heads up");
        let cases = read(&db);
        let case = cases[&key].as_ref().unwrap();
        let (event, name) = on_the_human(case).unwrap();
        let mut feed = Feed::new("boot".into());
        for _ in 0..FEED_SIZE + 7 {
            feed.add(Item::new(
                key,
                case,
                event,
                name,
                now(),
                serde_json::Value::Null,
            ));
        }
        let ids =
            |after| -> Vec<u64> { feed.after(after).items.iter().map(|item| item.id).collect() };
        assert_eq!(feed.latest(), 57);
        assert_eq!(ids(0), (8..=57).collect::<Vec<_>>());
        assert_eq!(ids(55), [56, 57]);
        assert!(ids(1000).is_empty());
        let page = serde_json::to_value(feed.after(1000)).unwrap();
        assert_eq!(page["items"], serde_json::json!([]));
    }
}
