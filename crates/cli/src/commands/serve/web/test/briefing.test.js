// Briefings in the inbox (IN-14): their own tab with its own count, kept
// out of the other filters, the tally and the title's count; the document
// view; and the form's answer.
import { test } from "node:test";
import assert from "node:assert/strict";
import { render } from "preact-render-to-string";
import { html } from "../static/html.ts";
import { ask } from "../static/ask.ts";
import { CaseView, response } from "../static/case.ts";
import { List, counts, icon, listed, parts, split, tally } from "../static/lists.ts";
import { under } from "../static/routes.ts";
import { answered, thread } from "../static/thread.ts";

const now = Date.parse("2026-10-31T12:00:00Z");
const at = (minutes) => new Date(now - minutes * 60000).toISOString();
const agent = { id: "a1", kind: "agent", name: "lead" };
const human = { id: "h1", kind: "human", name: "ryan" };

// A case of `kind` in `state`, opened `minutes` ago.
const make = (id, kind, state, urgency, minutes, extra = {}) => ({
  id, kind, state, urgency, title: `case ${id}`, worker: "w", opened_at: at(minutes), updated_at: at(minutes), events: [], revision: 1, amend_seq: 0, ...extra,
});
const briefing = (id, state, urgency, minutes, extra = {}) =>
  make(id, "briefing", state, urgency, minutes, { body: "## What changed\n\nThe parser.", landed: false, ...extra });

// The store as serve lists it: open and parked, then the others.
const waiting = [
  make("d1", "decision", "open", "blocking", 30),
  briefing("g1", "open", "blocking", 20),
  briefing("l1", "open", "blocking", 25, { landed: true }),
  briefing("g2", "open", "whenever", 40),
  make("s1", "signoff", "open", "today", 10),
];
const others = [
  briefing("a1", "answered", "whenever", 90, { answer: { answered_at: at(5) } }),
  briefing("a2", "answered", "whenever", 95, { answer: { answered_at: at(50) } }),
  briefing("p1", "pickedup", "whenever", 100, { answer: { answered_at: at(60) }, pickup: { picked_up_at: at(3) } }),
  briefing("c1", "closed", "whenever", 200, { close: { outcome: "merged" } }),
  briefing("w1", "withdrawn", "whenever", 300),
  make("x1", "decision", "answered", "today", 60, { answer: { answered_at: at(7) } }),
];

test("briefings leave the inbox and in flight; picked-up ones stay in their tab; closed or withdrawn ones join done", () => {
  const { inbox, done, briefings } = split(waiting, others);
  assert.deepEqual(inbox.map((c) => c.id), ["d1", "s1"]);
  assert.deepEqual(done.map((c) => c.id), ["c1", "w1", "x1"]);
  assert.deepEqual(briefings.map((c) => c.id), ["g1", "l1", "g2", "a1", "a2", "p1"]);
  assert.deepEqual(listed(inbox, done, "inflight", now, briefings).map((c) => c.id), ["x1"]);
  assert.deepEqual(listed(inbox, done, "done", now, briefings).map((c) => c.id), ["c1", "w1"]);
});

test("the tally, the title's count and the tab icon count no briefing", () => {
  const { inbox } = split(waiting, others);
  const t = tally(inbox);
  assert.deepEqual([t.blocking, t.today, t.whenever, t.waiting], [1, 1, 0, 2]);
  assert.equal(icon(t), "/favicon.svg?n=2&blocking=1");
  // Only briefings open: nothing waits in the inbox.
  assert.equal(icon(tally(split([briefing("g1", "open", "blocking", 1)], []).inbox)), "/favicon.svg");
});

test("the briefings tab counts the open briefings, and the other filters count none", () => {
  const { inbox, done, briefings } = split(waiting, others);
  assert.deepEqual(counts(inbox, done, briefings), { inbox: 2, inflight: 1, done: 2, all: 5, briefings: 3 });
});

test("the tab lists those waiting to land before those landed, then the answered ones, picked up or not, folded, newest answer first", () => {
  const { inbox, done, briefings } = split(waiting, others);
  const heads = parts(inbox, done, "briefings", now, briefings).map((p) => `${p.label}${p.folded ? " (folded)" : ""}: ${p.cases.map((c) => c.id).join(",")}`);
  assert.deepEqual(heads, ["Waiting to land: g1,g2", "Landed: l1", "Answered (folded): a1,a2,p1"]);
  // A pickup since does not move a briefing: p1 was picked up last, but answered first.
  assert.equal(parts(inbox, done, "briefings", now, briefings).at(-1).note, "2 not picked up");
  // The page opens the first open briefing, and moves through the open ones only.
  assert.deepEqual(listed(inbox, done, "briefings", now, briefings).map((c) => c.id), ["g1", "g2", "l1"]);
});

test("the tab draws its count and the folded group, open when it holds the case shown", () => {
  const { inbox, done, briefings } = split(waiting, others);
  const draw = (selected) => render(html`<${List} inbox=${inbox} done=${done} briefings=${briefings} show="briefings" selected=${selected} now=${now} />`);
  const page = draw("g1");
  assert.ok(page.includes('<a href="/?show=briefings" class="tab-briefings on" aria-current="true">briefings <span class="count">3</span></a>'), page);
  assert.ok(page.includes('<details class="group group-answered"><summary class="label">Answered <span class="count">3</span> <span class="note">· 2 not picked up</span></summary>'), page);
  // With every answer picked up, the heading says only the count.
  const picked = split([], [others[2]]);
  const all = render(html`<${List} inbox=${picked.inbox} done=${picked.done} briefings=${picked.briefings} show="briefings" selected="" now=${now} />`);
  assert.ok(all.includes('<summary class="label">Answered <span class="count">1</span></summary>'), all);
  assert.ok(!page.includes("no briefings to read."), page);
  assert.ok(page.includes("waiting to land</p>"), page);
  assert.ok(!page.includes("case d1"), page);
  // A picked-up briefing stays, so that its answer can still be withdrawn.
  const group = page.slice(page.indexOf('<details class="group group-answered">'));
  assert.ok(group.includes("case p1") && group.includes("picked up"), group);
  assert.ok(draw("a1").includes('<details class="group group-answered" open>'));
});

test("with no briefing open, the tab says so above the folded group", () => {
  const { inbox, done, briefings } = split([], others);
  const page = render(html`<${List} inbox=${inbox} done=${done} briefings=${briefings} show="briefings" selected="" now=${now} />`);
  assert.ok(page.indexOf("no briefings to read.") < page.indexOf("Answered <span"), page);
  assert.deepEqual(listed(inbox, done, "briefings", now, briefings), []);
});

test("a briefing's page is under the briefings tab until it is closed or withdrawn", () => {
  const filter = (state) => under({ kind: "briefing", state });
  assert.deepEqual(["open", "answered", "pickedup", "closed", "withdrawn"].map(filter), ["briefings", "briefings", "briefings", "done", "done"]);
  assert.equal(under({ kind: "signoff", state: "open" }), "inbox");
});

test("a briefing's card says whether it landed and how much it left out", () => {
  assert.equal(ask(briefing("g1", "open", "today", 1), null), "waiting to land");
  assert.equal(ask(briefing("l1", "open", "today", 1, { landed: true, left_out: ["a", "b"] }), null), "landed · 2 left out");
});

// A form's fields, with `extra` set.
const fields = (extra) => ({ choice: "", verdicts: {}, rowNotes: {}, signoff: "", briefing: "", file: [], stuck: "", text: "", note: "", ...extra });
const doc = briefing("g1", "open", "blocking", 1, { left_out: ["the docs", "a bench", "the TUI"] });

test("the form sends accept, with the ticked items to file in order, and a note when one is written", () => {
  assert.deepEqual(response(doc, fields({ briefing: "accept" }), false), { answer: { briefing: "accept" } });
  assert.deepEqual(response(doc, fields({ briefing: "accept", file: [3, 1], note: "  thanks " }), false), { answer: { note: "thanks", briefing: "accept", file: [1, 3] } });
});

test("the form sends a send back only with a reason, and nothing without a verdict", () => {
  assert.deepEqual(response(doc, fields({ briefing: "send_back", note: "   " }), false), { problem: "sending back needs a reason: say what to fix" });
  assert.deepEqual(response(doc, fields({ briefing: "send_back", note: "split the commit", file: [2] }), false), { answer: { note: "split the commit", briefing: "send_back", file: [2] } });
  assert.deepEqual(response(doc, fields({ file: [1] }), false), { problem: "choose accept or send back" });
});

const draw = (c) => render(html`<${CaseView} c=${c} now=${now} onSent=${() => {}} onCase=${() => {}} />`);

test("a briefing reads as a document: headline, landed mark, body, then the left-out items", () => {
  const page = draw(doc);
  const order = ["case g1</h1>", '<p class="landed-mark gated">waiting to land</p>', "What changed</h2>", '<h2 class="section" id="left-out-head">left out</h2><ol><li>the docs</li><li>a bench</li><li>the TUI</li></ol>'];
  const where = order.map((text) => page.indexOf(text));
  assert.ok(where.every((i, n) => i >= 0 && (n === 0 || i > where[n - 1])), `${where} ${page}`);
  assert.ok(draw({ ...doc, landed: true }).includes('<p class="landed-mark landed">landed</p>'));
});

test("a picked-up briefing's page offers to withdraw it, as an answered one's does", () => {
  for (const state of ["answered", "pickedup"]) {
    const page = draw({ ...doc, state, answer: { briefing: "accept", answered_at: at(5) } });
    assert.ok(page.includes('<form class="respond" data-write="withdraw"'), `${state}: ${page}`);
    assert.ok(!page.includes('data-write="answer"'), `${state}: ${page}`);
  }
});

test("the form has accept and send back, numbered, a tick box per left-out item, drop and the note", () => {
  const page = draw(doc);
  const form = page.slice(page.indexOf('data-write="answer"'), page.indexOf("</form>"));
  assert.ok(form.includes('value="accept" required aria-keyshortcuts="1"'), form);
  assert.ok(form.includes('value="send_back" required aria-keyshortcuts="2"'), form);
  assert.deepEqual([...form.matchAll(/type="checkbox" name="file" value="(\d)"/g)].map((m) => m[1]), ["1", "2", "3"]);
  assert.ok(form.includes('id="respond-drop"'), form);
  assert.ok(form.includes("note to the agent"), form);
  // No left-out items, no tick boxes.
  assert.ok(!draw({ ...doc, left_out: undefined }).includes('type="checkbox"'));
});

// A briefing sent back with an item filed, then amended with --reopen and a
// new left-out list: the events serve gives.
const events = [
  { seq: 1, author: "agent", event: "open", file: "", at: at(30), actor: agent, data: { body: "v1", landed: false, left_out: ["the docs", "a bench"] } },
  { seq: 2, author: "human", event: "answer", file: "", at: at(20), actor: human, data: { briefing: "send_back", note: "split it", file: [2] } },
  { seq: 3, author: "agent", event: "pickup", file: "", at: at(19), actor: agent, data: {} },
  { seq: 4, author: "agent", event: "amend", file: "", at: at(10), actor: agent, data: { body: "v2", reopen: true, landed: true, left_out: ["the TUI"] } },
];
const reopened = briefing("r1", "open", "today", 30, { body: "v2", landed: true, left_out: ["the TUI"], events, revision: 4, amend_seq: 4 });

test("the thread says a briefing's verdict and names the items filed from the list the answer saw", () => {
  const said = thread(reopened).map((s) => s.lines.map((l) => l.text));
  assert.deepEqual(said[1], ["by ryan", "briefing: send back", "note: split it", "file 2. a bench"]);
  assert.deepEqual(said[3], ["by lead", "reopened the case", "replaced the body", "landed", "replaced the left-out items: the TUI"]);
});

test("a reopened briefing is asked afresh, with the earlier answer in the thread", () => {
  const page = draw(reopened);
  assert.ok(!page.includes("your previous answer"), page);
  assert.ok(page.indexOf("v2") < page.indexOf('data-write="answer"'), page);
  assert.ok(page.indexOf('id="thread"') < page.indexOf("briefing: send back"), page);
});

test("a previous answer names the items it filed from the list it saw, not the list an amend put in its place", () => {
  const note = { seq: 5, author: "agent", event: "note", file: "", at: at(5), actor: agent, data: { body: "Done, see v2.", reopen: true } };
  const c = { ...reopened, events: [...events, note], revision: 5 };
  assert.deepEqual(answered(c, events[1].data), ["briefing: send back", "note: split it", "file 2. a bench"]);
});

test("an amend that splits one left-out item into two is still said in the thread", () => {
  const split = [
    { seq: 1, author: "agent", event: "open", file: "", at: at(30), actor: agent, data: { body: "v1", left_out: ["a\nb"] } },
    { seq: 2, author: "agent", event: "amend", file: "", at: at(20), actor: agent, data: { left_out: ["a", "b"] } },
  ];
  const said = thread(briefing("s1", "open", "today", 30, { left_out: ["a", "b"], events: split })).map((s) => s.lines.map((l) => l.text));
  assert.ok(said[1].includes("replaced the left-out items: a; b"), said[1]);
});

test("ticks numbered past the left-out list are not sent", () => {
  assert.deepEqual(response({ ...doc, left_out: ["only one"] }, fields({ briefing: "accept", file: [1, 3] }), false), { answer: { briefing: "accept", file: [1] } });
});
