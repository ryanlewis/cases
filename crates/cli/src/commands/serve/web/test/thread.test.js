// What each event of a case says in its thread (IN-02): `thread`, and
// `answered`, the lines of one answer. The cases follow describe.rs's tests,
// the CLI's `show` of the same thread; where the two differ, the test says so.
import { test } from "node:test";
import assert from "node:assert/strict";
import { answered, option, thread, who } from "../static/thread.ts";

const builder = { id: "a1", kind: "agent", name: "builder" };
const nameless = { id: "a2", kind: "agent" };
const gone = { id: "a3" };

// A case of `kind` with `events`, each `[event, data, extra]`, numbered
// from 1, and the case's own fields from `fields`.
function make(kind, events, fields = {}) {
  return {
    id: "aB3xQ",
    kind,
    state: "open",
    title: "T",
    ...fields,
    events: events.map(([event, data = {}, extra = {}], i) => ({ seq: i + 1, author: "agent", event, file: "", data, ...extra })),
  };
}

const decision = (events, fields) => make("decision", [["open", { title: "Pick", options: ["alpha", "beta"], recommend: 2 }], ...events], { options: ["alpha", "beta"], ...fields });
const question = (events, fields) => make("question", [["open", { title: "Q" }], ...events], fields);

// The text of each event's lines, in order.
const texts = (c) => thread(c).map((s) => s.lines.map((l) => l.text));
// The lines one answer `data` of case `c` gives in the thread.
const said = (c, data) => texts({ ...c, events: [...c.events, { seq: 99, event: "answer", data }] }).at(-1);

test("a choice names its option, and a number the case has no option for says nothing", () => {
  const c = decision([]);
  assert.deepEqual(said(c, { choice: 1 }), ["chose 1. alpha"]);
  assert.deepEqual(said(c, { choice: 2 }), ["chose 2. beta"]);
  assert.deepEqual(said(c, { choice: 3 }), []);
  assert.deepEqual(said(c, { choice: 0 }), []);
  assert.deepEqual(said(c, { choice: -1 }), []);
  assert.equal(option(c, 1), "alpha");
  assert.equal(option(c, 3), undefined);
  assert.equal(option(c, 0), undefined);
  assert.equal(option(c, -5), undefined);
  assert.equal(option(c, undefined), undefined);
  assert.equal(option({ ...c, options: undefined }, 1), undefined);
});

test("an answer says the first of its forms", () => {
  const d = decision([]);
  const q = question([]);
  assert.deepEqual(said(d, { choice: 1, other: true, signoff: "accept", text: "t", drop: true, ack: true }), ["chose 1. alpha"]);
  assert.deepEqual(said(d, { other: true, signoff: "accept", text: "t" }), ["chose other"]);
  assert.deepEqual(said(d, { signoff: "accept", briefing: "send_back", text: "t" }), ["signoff: accept"]);
  assert.deepEqual(said(d, { briefing: "send_back", text: "t" }), ["briefing: send back"]);
  assert.deepEqual(said(q, { text: "yes\nmaybe", drop: true }), ["reply: yes", "maybe"]);
  assert.deepEqual(said(d, { text: "do it", drop: true }), ["guidance: do it"]);
  assert.deepEqual(said(d, { drop: true, ack: true }), ["drop"]);
  assert.deepEqual(said(d, { ack: true }), ["acknowledged"]);
  assert.deepEqual(said(d, {}), []);
});

test("an answer adds row verdicts, `reject` as don't run, then its note", () => {
  const rows = [
    { id: "a", verdict: "approve" },
    { id: "b", verdict: "reject", note: "too risky" },
    { id: "c", verdict: "hold", note: "later" },
    { id: "d", verdict: "reject" },
  ];
  assert.deepEqual(said(question([]), { rows, note: "overall\nfine" }), ["[a] approve", "[b] don't run: too risky", "[c] hold: later", "[d] don't run", "note: overall", "fine"]);
  // The verdicts follow the first form's line.
  assert.deepEqual(said(question([]), { ack: true, rows: [rows[0]] }), ["acknowledged", "[a] approve"]);
});

test("an answer through the thread", () => {
  assert.deepEqual(texts(decision([["answer", { choice: 2, note: "ok" }]])), [[], ["chose 2. beta", "note: ok"]]);
});

test("events without text have no lines", () => {
  // The thread reads each event on its own, not whether the store would
  // have taken it after the one before, so one case holds them all.
  const c = question([["pickup"], ["withdraw"], ["park"], ["resume"], ["note"], ["close"]]);
  assert.deepEqual(texts(c), [[], [], [], [], [], [], []]);
  // A note and a close leave their body and outcome blank too.
  assert.deepEqual(thread(c).map((s) => s.markdown), ["", "", "", "", "", "", ""]);
  // The thread reads an event with no data as one with none of its fields.
  assert.deepEqual(texts(question([["pickup", null]])), [[], []]);
  assert.deepEqual(thread({ ...question([]), events: undefined }), []);
});

test("park, withdraw and pickup say their text", () => {
  const c = question([["pickup", { by: "worker-1" }], ["park", { note: "later" }], ["withdraw", { reason: "moot" }]]);
  assert.deepEqual(texts(c).slice(1), [["by worker-1"], ["note: later"], ["reason: moot"]]);
});

test("a note's body and a close's outcome are its markdown; a close's links are its lines", () => {
  const c = question([
    ["answer", { text: "a" }],
    ["pickup", { by: "w" }],
    ["note", { body: "  working\non it  ", reopen: true }],
    ["answer", { text: "b" }],
    ["pickup"],
    ["close", { outcome: "done", links: ["https://a", "https://b"] }],
  ]);
  // describe.rs says the note's body and the close's outcome as lines; the
  // inbox draws them as markdown instead.
  assert.deepEqual(texts(c), [[], ["reply: a"], ["by w"], ["reopened the case"], ["reply: b"], [], ["https://a", "https://b"]]);
  assert.deepEqual(thread(c).map((s) => s.markdown), ["", "", "", "  working\non it  ", "", "", "done"]);
});

test("actors are named by their name, their id when the row has none, or as deleted", () => {
  const by = (actor) => texts(make("question",[["open", { title: "Q" }, { actor }]]))[0];
  assert.deepEqual(by(builder), ["by builder"]);
  // describe.rs says nothing of an actor whose row has no name; the inbox
  // names it by its id.
  assert.deepEqual(by(nameless), ["by a2"]);
  assert.deepEqual(by(gone), ["by deleted user"]);
  assert.deepEqual(by(undefined), []);
  assert.equal(who({ id: "x", kind: "", name: "" }), "deleted user");
});

test("a pickup by its own actor has one line", () => {
  const pickup = (by) => texts(question([["answer", { text: "a" }], ["pickup", { by }, { actor: builder }]]))[2];
  assert.deepEqual(pickup("builder"), ["by builder"]);
  assert.deepEqual(pickup("other"), ["by builder", "by other"]);
  // Only a pickup's `by` stands in for its actor's line.
  assert.deepEqual(texts(question([["park", { by: "builder" }, { actor: builder }]]))[1], ["by builder"]);
});

test("a message says who it went to: its worker, then its labels", () => {
  const to = (data) => texts(make("message", [["open", { title: "M", ...data }]]))[0];
  assert.deepEqual(to({ worker: "w1", labels: ["x", "y"] }), ["sent to w1 or label x or label y"]);
  assert.deepEqual(to({ labels: ["x"] }), ["sent to label x"]);
  assert.deepEqual(to({ worker: "w1" }), ["sent to w1"]);
  assert.deepEqual(to({}), []);
  // Only a message says it; a question with a worker does not.
  assert.deepEqual(texts(make("question", [["open", { title: "Q", worker: "w1" }]]))[0], []);
});

test("a note or amend after an answer reopens the case, once", () => {
  const c = question([
    ["answer", { text: "a" }],
    ["amend", { labels: ["l"], reopen: true }],
    ["answer", { text: "b" }],
    ["pickup"],
    ["amend", { labels: ["m"], reopen: true }],
    ["amend", { labels: ["n"] }],
  ]);
  assert.deepEqual(texts(c), [[], ["reply: a"], ["reopened the case", "added label: l"], ["reply: b"], [], ["reopened the case", "added label: m"], ["added label: n"]]);
  // An amend of an open case does not reopen it.
  assert.deepEqual(texts(question([["amend", { labels: ["l"] }]]))[1], ["added label: l"]);
  // The inbox reads whether the case stands answered from the answers and
  // withdraws before an event, not from its `from` as describe.rs does: a
  // withdraw clears it, and nothing else but a note or amend does. The
  // store takes neither thread below; they pin that reading.
  assert.deepEqual(texts(question([["answer", { text: "a" }], ["withdraw"], ["note", { body: "n" }]]))[3], []);
  // A park and a close say nothing of it, and the note after them reopens
  // the case; describe.rs, reading the note's `from`, would not say so.
  assert.deepEqual(texts(question([["answer", { text: "a" }], ["park"], ["close"], ["note"]])).slice(2), [[], [], ["reopened the case"]]);
  // The reopening line follows the actor's.
  assert.deepEqual(texts(question([["answer", { text: "a" }], ["note", {}, { actor: builder }]]))[2], ["by builder", "reopened the case"]);
});

const approval = { title: "Old", body: "old body", rows: [{ id: "r0", label: "Zero" }], links: ["https://gone"], context: "old context" };

test("an amend says each change, keeping the replaced body and context on its line", () => {
  const c = make("approval", [
    ["open", approval],
    ["amend", { title: "New", body: "new body", rows: [{ id: "r1", label: "Row" }], drop_links: ["https://gone"], links: ["https://new"], labels: ["l1"], context: "new context" }],
  ]);
  const lines = thread(c)[1].lines;
  assert.deepEqual(lines, [
    { text: "replaced the title: New", previous: null },
    { text: "replaced the body", previous: { body: "old body" } },
    { text: "added row [r1] Row", previous: null },
    { text: "dropped link: https://gone", previous: null },
    { text: "added link: https://new", previous: null },
    { text: "added label: l1", previous: null },
    { text: "replaced the context: new context", previous: { context: "old context" } },
  ]);
});

test("an amend of a decision adds options and recommends", () => {
  const c = decision([["amend", { options: ["gamma"], recommend: 3 }], ["amend", { recommend: 1 }], ["amend", { recommend: 9 }]], { options: ["alpha", "beta", "gamma"] });
  assert.deepEqual(texts(c).slice(1), [["added option: gamma", "recommended 3. gamma"], ["recommended 1. alpha"], ["recommended 9."]]);
});

test("an amend says only what differs", () => {
  const c = make("decision", [
    ["open", { title: "T", body: "B", options: ["a", "b"], recommend: 2, context: "C", landed: true, left_out: ["x", "y"] }],
    ["amend", { title: "T", body: "B", recommend: 2, context: "C", landed: true, left_out: ["x", "y"], labels: ["l"] }],
    // An empty left-out list leaves the items as they were.
    ["amend", { title: "", body: "", recommend: 0, context: "", left_out: [] }],
  ]);
  assert.deepEqual(texts(c).slice(1), [["added label: l"], []]);
});

test("a replaced text is compared with the last one", () => {
  const c = make("question", [
    ["open", { title: "A", body: "one", context: "c1" }],
    ["amend", { title: "B", body: "two", context: "c2" }],
    // Back to the open's text: it differs from the last, so it is a change.
    ["amend", { title: "A", body: "one", context: "c1" }],
  ]);
  assert.deepEqual(thread(c)[2].lines, [
    { text: "replaced the title: A", previous: null },
    { text: "replaced the body", previous: { body: "two" } },
    { text: "replaced the context: c1", previous: { context: "c2" } },
  ]);
});

test("a blank previous text is not kept", () => {
  const c = make("question", [["open", { title: "T", body: "  \n ", context: "\t" }], ["amend", { body: "now", context: "ctx" }]]);
  assert.deepEqual(thread(c)[1].lines, [
    { text: "replaced the body", previous: null },
    { text: "replaced the context: ctx", previous: null },
  ]);
  // Nor is one the open left out.
  const bare = make("question", [["open", {}], ["amend", { title: "T", body: "now", context: "ctx" }]]);
  assert.deepEqual(thread(bare)[1].lines, [
    { text: "replaced the title: T", previous: null },
    { text: "replaced the body", previous: null },
    { text: "replaced the context: ctx", previous: null },
  ]);
});

test("a replaced text over many lines keeps the previous one on its last", () => {
  const c = make("question", [["open", { context: "before" }], ["amend", { context: "x\ny" }]]);
  assert.deepEqual(thread(c)[1].lines, [
    { text: "replaced the context: x", previous: null },
    { text: "y", previous: { context: "before" } },
  ]);
});

test("a title an open gives as other than text counts as none", () => {
  const c = make("question", [["open", { title: 7 }], ["amend", { title: "T" }]]);
  assert.deepEqual(texts(c)[1], ["replaced the title: T"]);
});

test("a body the record leaves out is the case's own", () => {
  const c = make("question", [["open", {}, { body_in_case: true }], ["amend", { body: "two" }], ["amend", {}, { body_in_case: true }]], { body: "now" });
  assert.deepEqual(thread(c).slice(1).map((s) => s.lines), [
    [{ text: "replaced the body", previous: { body: "now" } }],
    [{ text: "replaced the body", previous: { body: "two" } }],
  ]);
  // A case with no body of its own gives an empty one.
  const none = make("question", [["open", { body: "b" }], ["amend", {}, { body_in_case: true }]]);
  assert.deepEqual(texts(none)[1], []);
});

test("an amend of a briefing says it landed or not, and its new left-out items", () => {
  const c = make("briefing", [
    ["open", { body: "v1", left_out: ["a"] }],
    ["amend", { landed: true, left_out: ["a", "b"] }],
    ["amend", { landed: false, left_out: ["a", "c"] }],
    ["amend", { landed: false, left_out: ["a", "c"] }],
  ]);
  assert.deepEqual(texts(c).slice(1), [["landed", "replaced the left-out items: a; b"], ["not landed", "replaced the left-out items: a; c"], []]);
});

test("an answer files a briefing's items from the list it saw", () => {
  const c = make("briefing", [
    ["open", { left_out: ["the docs", "a bench"] }],
    ["answer", { briefing: "send_back", file: [2, 1, 3, 0] }],
    ["amend", { left_out: ["the TUI"], reopen: true }],
    ["answer", { briefing: "accept", file: [1] }],
  ]);
  const said = texts(c);
  assert.deepEqual(said[1], ["briefing: send back", "file 2. a bench", "file 1. the docs", "file 3.", "file 0."]);
  assert.deepEqual(said[3], ["briefing: accept", "file 1. the TUI"]);
});

test("answered says an answer's lines as the thread does, from the list it saw", () => {
  const c = make("briefing", [
    ["open", { left_out: ["the docs", "a bench"] }],
    ["amend", { left_out: [] }],
    ["amend", { labels: ["l"] }],
    ["answer", { briefing: "send_back", note: "n\nm", file: [2] }],
    ["amend", { left_out: ["the TUI"], reopen: true }],
    ["answer", { briefing: "accept", file: [1] }],
  ], { left_out: ["the TUI"] });
  assert.deepEqual(answered(c, c.events[3].data), ["briefing: send back", "note: n", "m", "file 2. a bench"]);
  assert.deepEqual(answered(c, c.events[5].data), ["briefing: accept", "file 1. the TUI"]);
  // An answer not among the events reads the case's own left-out items.
  assert.deepEqual(answered(c, { file: [1] }), ["file 1. the TUI"]);
  assert.deepEqual(answered({ ...c, left_out: undefined }, { file: [1] }), ["file 1."]);
  assert.deepEqual(answered({ ...c, events: undefined, left_out: undefined }, { ack: true }), ["acknowledged"]);
  // BUG (follow-up): an earlier event with no data stops answered, which
  // reads its fields without the guard thread has; thread says the answer.
  const nodata = make("briefing", [["open", null], ["answer", { file: [1] }]]);
  assert.deepEqual(texts(nodata)[1], ["file 1."]);
  assert.throws(() => answered(nodata, nodata.events[1].data), TypeError);
  // An open with no left-out items gives an empty list.
  const bare = make("briefing", [["open", {}], ["answer", { file: [1] }]]);
  assert.deepEqual(answered(bare, bare.events[1].data), ["file 1."]);
});

test("a line is trimmed and split at each newline", () => {
  const c = question([["park", { note: "\n  a\nb  \n" }], ["withdraw", { reason: "x\n\ny" }]]);
  // The text is trimmed as a whole, not line by line, as describe.rs does.
  assert.deepEqual(texts(c).slice(1), [["note: ", "  a", "b"], ["reason: x", "", "y"]]);
  // describe.rs says nothing of a blank close link and drops a "\r" before a
  // newline; the inbox keeps an empty line, and the "\r".
  assert.deepEqual(texts(question([["close", { links: ["   ", "a\r\nb"] }]]))[1], ["", "a\r", "b"]);
});

test("an event the thread does not know says nothing, whatever its name", () => {
  for (const name of ["bogus", "toString", "constructor", "__proto__", "hasOwnProperty"]) {
    const c = question([[name, { body: "b", outcome: "o" }]]);
    assert.deepEqual(thread(c)[1], { event: c.events[1], lines: [], markdown: "" }, name);
  }
});
