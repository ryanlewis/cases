import { test } from "node:test";
import assert from "node:assert/strict";
import { render } from "preact-render-to-string";
import { html } from "../static/html.ts";
import { ask, groups, ordered } from "../static/ask.ts";
import { followUp } from "../static/followup.ts";
import { List, listed } from "../static/lists.ts";
import { decision, steps } from "./case.js";

const { open, answer, pickup, note } = steps;

// A case of `kind`, open, with `fields`.
const of = (kind, fields = {}) => ({ id: "x", state: "open", kind, urgency: "today", title: "t", events: [], ...fields });

test("a decision says how many options it has, and the one the agent recommends", () => {
  const options = ["a", "b", "c", "d"];
  assert.equal(ask(of("decision", { options, recommend: 1, body: "Why: cheapest." }), null), "4 options · recommends 1");
  assert.equal(ask(of("decision", { options: ["a"] }), null), "1 option");
});

test("a Recommendation line in the body is text, not the recommend", () => {
  assert.equal(ask(of("decision", { options: ["a", "b"], body: "**Recommendation: 2.** Cheapest." }), null), "2 options");
});

test("an approval says how many scripts, or items, it asks to approve", () => {
  const row = (id, script) => ({ id, label: id, ...(script ? { script } : {}) });
  assert.equal(ask(of("approval", { rows: [row("a", "x"), row("b", "y"), row("c", "z")] }), null), "3 scripts to approve");
  assert.equal(ask(of("approval", { rows: [row("a")] }), null), "1 item to approve");
});

test("a signoff counts its body's ticked checks, or says what it asks", () => {
  const body = "Checks:\n\n- [x] one\n- [x] two\n* [X] three\n- [x] four\n- [ ] five";
  assert.equal(ask(of("signoff", { body }), null), "4 of 5 checks");
  assert.equal(ask(of("signoff", { body: "Ready to ship." }), null), "accept or request changes");
});

test("a question, stuck or fyi says its body's first line, or its context", () => {
  assert.equal(ask(of("question", { body: "\n\n## Which **timezone** for `digest`?\n\nMore." }), null), "Which timezone for digest?");
  assert.equal(ask(of("stuck", { body: "- sessions.account_id is null\n- more" }), null), "sessions.account_id is null");
  assert.equal(ask(of("stuck", { body: '```rust\npanic!("boom");\n```' }), null), 'panic!("boom");');
  assert.equal(ask(of("fyi", { context: "Flags are on." }), null), "Flags are on.");
  assert.equal(ask(of("fyi"), null), "");
});

test("a parked case says its park note", () => {
  assert.equal(ask(of("stuck", { state: "parked", body: "x", park: { note: " Ask again after Monday " } }), null), "parked: Ask again after Monday");
  assert.equal(ask(of("decision", { state: "parked", options: ["a"] }), null), "parked");
});

test("a follow-up says what the last answer chose", () => {
  const chose = (data) => ({ ...answer, data });
  const c = of("decision", { options: ["a", "b"] });
  assert.equal(ask(c, { answer: chose({ choice: 2 }), notes: [note] }), "follow-up · you chose 2");
  assert.equal(ask(c, { answer: chose({ other: true, note: "neither" }), notes: [note] }), "follow-up · you chose other");
  assert.equal(ask(of("question"), { answer: chose({ text: "yes" }), notes: [note] }), "follow-up · you answered");
  // The follow-up detection's own case: answered other, then the agent's note.
  const reopened = decision("open", open, answer, pickup, note);
  assert.equal(ask(reopened, followUp(reopened)), "follow-up · you chose other");
});

const inbox = [
  { id: "a", state: "parked", urgency: "blocking" },
  { id: "b", state: "open", urgency: "blocking" },
  { id: "c", state: "open", urgency: "whenever" },
  { id: "d", state: "open", urgency: "blocking" },
  { id: "e", state: "parked", urgency: "whenever" },
  { id: "f", state: "open", urgency: "whenever" },
].map((c) => of("fyi", { ...c, title: `case ${c.id}`, body: `about ${c.id}` }));

test("the inbox groups under Blocking, Today, Whenever and Parked, leaving out empty groups", () => {
  assert.deepEqual(
    groups(inbox).map((g) => [g.label, g.cases.map((c) => c.id)]),
    [
      ["Blocking", ["b", "d"]],
      ["Whenever", ["c", "f"]],
      ["Parked", ["a", "e"]],
    ],
  );
  assert.deepEqual(ordered(inbox).map((c) => c.id), ["b", "d", "c", "f", "a", "e"]);
});

test("the inbox draws its groups' headings and cards in that order, for j and k", () => {
  const page = render(html`<${List} inbox=${inbox} done=${[]} show="inbox" selected="d" now=${Date.now()} />`);
  const headings = [...page.matchAll(/<h2 id="group-(\w+)"[^>]*>(\w+) <span class="count">(\d+)<\/span>/g)].map((m) => m.slice(1).join(" "));
  assert.deepEqual(headings, ["blocking Blocking 2", "whenever Whenever 2", "parked Parked 2"]);
  const cards = [...page.matchAll(/<h3>case (\w)<\/h3>/g)].map((m) => m[1]);
  assert.deepEqual(cards, ["b", "d", "c", "f", "a", "e"]);
  assert.match(page, /<p class="ask">about b<\/p>/);
  assert.match(page, /<p class="ask parked">parked<\/p>/);
});

test("the page opens the first case, and the next after a send, in the grouped order", () => {
  // The page opens listed(...)[0], and after a send the case after the one sent.
  assert.deepEqual(listed(inbox, [], "inbox", Date.now()).map((c) => c.id), ordered(inbox).map((c) => c.id));
});
