import { test } from "node:test";
import assert from "node:assert/strict";
import { render } from "preact-render-to-string";
import { html } from "../static/html.ts";
import { CaseView, drawnFrom, redrawn, response, verdictOf } from "../static/case.ts";
import { body, decision, steps } from "./case.js";
import { thread } from "../static/thread.ts";

const { open, answer, pickup, note } = steps;
const now = Date.parse("2026-09-25T13:45:00Z");
const draw = (c) => render(html`<${CaseView} c=${c} now=${now} onSent=${() => {}} onCase=${() => {}} />`);

// Where `text` first is in `page`, which must have it.
function at(page, text) {
  const i = page.indexOf(text);
  assert.notEqual(i, -1, `the page does not say ${JSON.stringify(text)}`);
  return i;
}

// The options of `page`'s answer form that `mark` marks, by their value.
function markedIn(page, mark) {
  const form = page.slice(at(page, 'data-write="answer"'), at(page, "</form>"));
  const choices = form.match(/<label class="choice">(?:(?!<\/label>).)*<\/label>/g) || [];
  return choices.filter((choice) => choice.includes(mark)).map((choice) => /value="(\w+)"/.exec(choice)[1]);
}

test("a reopened case shows the agent's note above its previous answer and the body", () => {
  const page = draw(decision("open", open, answer, pickup, note));
  const followed = at(page, "Follow-up: a watch or a print, then?");
  assert.ok(at(page, "new since your answer") < followed);
  const previous = at(page, "your previous answer");
  assert.ok(followed < previous);
  assert.ok(previous < at(page, "chose other"));
  assert.ok(previous < at(page, "note: option 2 is out"));
  assert.ok(previous < at(page, "picked up by gift-agent"));
  const original = at(page, '<details class="original">');
  assert.ok(previous < original);
  assert.ok(original < page.indexOf(body, original));
  // The form is there to answer again, with the last pick marked.
  const form = at(page, 'data-write="answer"');
  assert.ok(original < form);
  assert.ok(form < at(page, "other, see note"));
  assert.equal(page.match(/your last pick/g)?.length, 1);
  assert.ok(at(page, "other, see note") < at(page, "your last pick"));
});

// An amend that reopens the case: a new title and body, the old link
// dropped and a new one added (cases 6M6gx, AV8s1).
const reopening = {
  seq: 4,
  author: "agent",
  event: "amend",
  file: "",
  at: "2026-09-25T13:30:00Z",
  data: {
    title: "A watch or a print?",
    body: "Two left: a watch or a print.",
    drop_links: ["https://example.com/shop/1"],
    links: ["https://example.com/shop/2"],
    reopen: true,
  },
};

// The case after the reopening amend, as the API gives it.
function reopened() {
  const first = { ...open, data: { ...open.data, title: "Which way for the anniversary present?", links: ["https://example.com/shop/1"] } };
  return {
    ...decision("open", first, answer, pickup, reopening),
    title: "A watch or a print?",
    body: "Two left: a watch or a print.",
    links: ["https://example.com/shop/2"],
    amend_seq: 4,
  };
}

test("a case an amend reopened is asked afresh: its new title and body first, the old answer in the thread", () => {
  const page = draw(reopened());
  assert.equal(page.includes("follow-up"), false);
  assert.equal(page.includes("your previous answer"), false);
  assert.equal(page.includes("your last pick"), false);
  const title = at(page, "A watch or a print?</h1>");
  assert.ok(title < at(page, "Two left: a watch or a print."));
  assert.ok(at(page, "Two left: a watch or a print.") < at(page, 'data-write="answer"'));
  const thread = at(page, 'id="thread"');
  assert.ok(thread < page.indexOf("chose other", thread));
  assert.ok(thread < page.indexOf("reopened the case", thread));
});

test("the thread says an amend reopened the case, replaced its title and dropped a link", () => {
  const said = thread(reopened()).find((s) => s.event === reopening);
  assert.deepEqual(
    said.lines.map((line) => line.text),
    [
      "reopened the case",
      "replaced the title: A watch or a print?",
      "replaced the body",
      "dropped link: https://example.com/shop/1",
      "added link: https://example.com/shop/2",
    ],
  );
});

test("the thread says a note reopened the case, with reopen or, as notes stored before it, without", () => {
  for (const n of [note, { ...note, data: { ...note.data, reopen: true } }]) {
    const said = thread(decision("open", open, answer, pickup, n)).find((s) => s.event === n);
    assert.deepEqual(said.lines.map((line) => line.text), ["by gift-agent", "reopened the case"]);
  }
  const onOpen = { ...note, seq: 2 };
  const said = thread(decision("open", open, onOpen)).find((s) => s.event === onOpen);
  assert.deepEqual(said.lines.map((line) => line.text), ["by gift-agent"]);
});

test("the option chosen last is the one marked", () => {
  const chose = { ...answer, data: { choice: 3 } };
  const page = draw(decision("open", open, chose, pickup, note));
  assert.ok(at(page, "chose 3. a print") < at(page, '<details class="original">'));
  assert.deepEqual(markedIn(page, ">your last pick<"), ["3"]);
});

test("a case not reopened by a note shows its body first, as before", () => {
  for (const c of [decision("open", open), decision("pickedup", open, answer, pickup)]) {
    const page = draw(c);
    assert.equal(page.includes("follow-up"), false);
    assert.equal(page.includes("your last pick"), false);
    assert.ok(page.indexOf(body, at(page, "</h1>")) < at(page, 'id="thread"'));
  }
});

test("the refusal line and the form sit in one box, after the body and before the thread, so a wide page can hold them in view", () => {
  for (const state of ["open", "parked"]) {
    const page = draw(decision(state, open));
    const box = at(page, '<div class="answer">');
    assert.ok(page.indexOf(body, at(page, "</h1>")) < box);
    assert.ok(box < at(page, 'id="refusal"'));
    assert.ok(at(page, 'id="refusal"') < at(page, '<form class="respond"'));
    assert.ok(at(page, "</form>") < page.indexOf("</div>", at(page, "</form>")));
    assert.ok(page.indexOf("</div>", at(page, "</form>")) < at(page, 'id="thread"'));
  }
});

test("an approval row's label is its title: the legend that names the group, above its note", () => {
  const label = "GitHub follow-ups ×53 → Notifications/GitHub";
  const rows = [{ id: "r1", label, note: "Moves them out of the inbox.", link: "", script: "mv github/* notifications/github/" }];
  const page = draw({ ...decision("open", open), kind: "approval", options: undefined, rows });
  const title = at(page, `<legend class="row-title">${label}</legend>`);
  assert.ok(at(page, '<fieldset class="row">') < title);
  assert.ok(title < at(page, "Moves them out of the inbox."));
  assert.equal(page.match(/class="row-title"/g)?.length, 1);
});

// The note of `row` as the page draws it, in its paragraph.
function rowNote(row) {
  const page = draw({ ...decision("open", open), kind: "approval", options: undefined, rows: [{ id: "r1", label: "a row", link: "", script: "true", ...row }] });
  return /<p class="context row-note">(.*?)<\/p>/s.exec(page.slice(at(page, 'class="row-title"')))[1];
}

test("an approval row's note is inline markdown: a link to its PR is a link, which opens in a new tab", () => {
  assert.equal(
    rowNote({ note: "Bumps `vitest`; see [PR 12](https://github.com/o/r/pull/12), *major*." }),
    'Bumps <code>vitest</code>; see <a href="https://github.com/o/r/pull/12" target="_blank" rel="noopener noreferrer">PR 12</a>, <em>major</em>.',
  );
  assert.equal(rowNote({ note: "https://github.com/o/r/pull/12" }), '<a href="https://github.com/o/r/pull/12" target="_blank" rel="noopener noreferrer">https://github.com/o/r/pull/12</a>');
});

test("an approval row's note links only what the page may link to: a javascript: or relative target is text", () => {
  for (const href of ["javascript:alert(1)", "JaVa&#9;script:alert(1)", "data:text/html,x", "../land.sh", "/cases/abc", "land.sh"]) {
    assert.equal(rowNote({ note: `run [it](${href})` }), "run it", href);
  }
});

test("an approval row's note draws no raw HTML, and no blocks", () => {
  const note = rowNote({ note: 'a <b>b</b> <img src=x onerror="alert(1)"> <a href="https://x.test">c</a>' });
  assert.doesNotMatch(note, /<(b|img|a)\b/);
  assert.equal(rowNote({ note: "# not a heading" }), "# not a heading");
  assert.equal(rowNote({ note: "- not a list" }), "- not a list");
});

test("a plain note on an approval row reads as it did", () => {
  assert.equal(rowNote({ note: "Moves them out of the inbox, and don't undo it." }), "Moves them out of the inbox, and don&#39;t undo it.");
  assert.equal(rowNote({ note: "a & b < c" }), "a &amp; b &lt; c");
});

test("a case:ID in the thread's text, a note's body or a row's note links to that case's page", () => {
  // A thread line is plain text, not markdown: a backtick in it is a
  // character like any other.
  const said = { ...answer, data: { other: true, note: "as case:eba6l said, not showcase:abcde or `case:x1y2z`" } };
  const asked = { ...note, data: { body: "See case:AbC12." } };
  const page = draw(decision("open", open, said, pickup, asked));
  const thread = page.slice(at(page, 'id="thread"'));
  assert.ok(thread.includes('<p>note: as <a class="case-ref" href="/cases/eba6l">case:eba6l</a> said, not showcase:abcde or `<a class="case-ref" href="/cases/x1y2z">case:x1y2z</a>`</p>'), thread);
  assert.ok(thread.includes('<p>See <a href="/cases/AbC12">case:AbC12</a>.</p>'), thread);
  // What is not a link in a thread line is text, escaped.
  const risky = { ...answer, data: { text: `<b>x</b> & "q" <case:eba6l> <img src=x onerror=alert(1)>` } };
  const q = draw({ ...decision("answered", open, risky), kind: "question", options: undefined });
  assert.ok(q.includes('<p>reply: &lt;b>x&lt;/b> &amp; &quot;q&quot; &lt;<a class="case-ref" href="/cases/eba6l">case:eba6l</a>> &lt;img src=x onerror=alert(1)></p>'), q.slice(q.indexOf('id="thread"')));
  const rows = [{ id: "r1", label: "case:eba6l", note: "after case:eba6l", link: "", script: "true" }];
  const approval = draw({ ...decision("open", open), kind: "approval", options: undefined, rows });
  assert.ok(approval.includes('<legend class="row-title">case:eba6l</legend>'), "a label is text");
  assert.ok(approval.includes('<p class="context row-note">after <a href="/cases/eba6l">case:eba6l</a></p>'), approval);
});

test("a body that ends with a case:ID leaves the thread's case links as they are", () => {
  // Markdown is drawn before the thread, and both read the same regex.
  const opened = { ...open, data: { ...open.data, body: "Which way? It follows case:AbC12" } };
  const said = { ...answer, data: { other: true, note: "case:eba6l said so" } };
  const page = draw({ ...decision("open", opened, said), body: opened.data.body });
  assert.ok(page.includes('<p>note: <a class="case-ref" href="/cases/eba6l">case:eba6l</a> said so</p>'), page.slice(page.indexOf('id="thread"')));
});

// The number keys' hints on a form's choices, in page order, as [value, key].
function numbered(page) {
  const form = page.slice(at(page, 'data-write="answer"'), at(page, "</form>"));
  return [...form.matchAll(/<label class="choice"><input[^>]*value="([^"]*)"[^>]*>(?:(?!<\/label>).)*?(?:<span class="key-hint" aria-hidden="true"><kbd>(\d)<\/kbd><\/span>)?<\/label>/g)].map((m) => [m[1], m[2]]);
}

test("each choice of a decision shows the number key that picks it, hidden from screen readers, which its radio names", () => {
  const page = draw(decision("open", open));
  assert.deepEqual(numbered(page), [["1", "1"], ["2", "2"], ["3", "3"], ["other", "4"]]);
  assert.ok(page.includes('<input type="radio" name="choice" value="2" required aria-keyshortcuts="2"'), page);
});

test("a decision's choices past the ninth have no key, and show none", () => {
  const options = Array.from({ length: 10 }, (_, i) => `option ${i + 1}`);
  const keys = numbered(draw({ ...decision("open", open), options })).map(([, key]) => key);
  assert.deepEqual(keys, ["1", "2", "3", "4", "5", "6", "7", "8", "9", undefined, undefined]);
});

test("a sign-off and a stuck case number their two choices, and each row of an approval its three", () => {
  assert.deepEqual(numbered(draw({ ...decision("open", open), kind: "signoff", options: undefined })), [["accept", "1"], ["changes", "2"]]);
  assert.deepEqual(numbered(draw({ ...decision("open", open), kind: "stuck", options: undefined })), [["text", "1"], ["drop", "2"]]);
  const rows = ["r1", "r2"].map((id) => ({ id, label: `row ${id}`, link: "", script: "true" }));
  const approval = draw({ ...decision("open", open), kind: "approval", options: undefined, rows });
  const verdicts = [["approve", "1"], ["hold", "2"], ["reject", "3"]];
  assert.deepEqual(numbered(approval), [...verdicts, ...verdicts]);
  assert.ok(approval.includes('<input type="radio" name="verdict.r2" value="hold" required aria-keyshortcuts="2"'), approval);
});

test("an approval's properties list the keys that give a row its verdict; other kinds' do not", () => {
  const rows = [{ id: "r1", label: "a row", link: "", script: "true" }];
  const verdicts = "<dt>verdict</dt><dd><kbd>1</kbd> <kbd>2</kbd> <kbd>3</kbd></dd>";
  const approval = draw({ ...decision("open", open), kind: "approval", options: undefined, rows });
  assert.ok(approval.includes(verdicts), approval);
  assert.equal(approval.includes("<dt>choose</dt>"), false);
  assert.equal(draw(decision("open", open)).includes(verdicts), false);
});

test("the send button shows the send keys for this system, hidden from screen readers, and names them", () => {
  const send = (kind) => {
    const page = draw({ ...decision("open", open), kind });
    return page.slice(at(page, '<button type="submit" class="primary" id="respond-send"'), page.indexOf("</button>", at(page, 'id="respond-send"')));
  };
  // node says what system it runs on as a browser does.
  const mod = /Mac/.test(navigator.platform) ? "⌘" : "Ctrl";
  for (const [kind, word] of [["decision", "send"], ["fyi", "acknowledge"]]) {
    assert.equal(send(kind), `<button type="submit" class="primary" id="respond-send" aria-keyshortcuts="Meta+Enter Control+Enter">${word} <span class="key-hint" aria-hidden="true"><kbd>${mod}</kbd><kbd>↵</kbd></span>`);
  }
});

test("a case an agent has, answered or picked up, offers the human's withdraw, folded, with a reason", () => {
  const withdrawn = { seq: 4, author: "human", event: "withdraw", file: "", data: { reason: "its session died" } };
  for (const c of [decision("answered", open, answer), decision("pickedup", open, answer, pickup)]) {
    const page = draw(c);
    const form = page.slice(at(page, '<form class="respond" data-write="withdraw"><details class="withdraw">'), at(page, "</form>"));
    assert.ok(at(form, "<summary>withdraw this case</summary>") < at(form, '<textarea name="reason"'), form);
    assert.ok(form.includes('<button type="submit" id="respond-withdraw">withdraw</button>'), form);
    // It is the case's only form. It sits under the body and above the
    // thread, but outside the answer box, which holds only the refusal line:
    // on a wide page the case keeps its one column, as a case with no form
    // does, rather than holding the folded withdraw in view beside the body.
    assert.equal(page.match(/<form /g)?.length, 1);
    const box = at(page, '<div class="answer">');
    assert.ok(page.indexOf(body, at(page, "</h1>")) < box);
    assert.ok(page.indexOf("</div>", box) < at(page, 'data-write="withdraw"'));
    assert.ok(at(page, 'data-write="withdraw"') < at(page, 'id="thread"'));
  }
  const parked = { ...decision("parked", open), kind: "stuck", options: undefined };
  for (const c of [decision("open", open), parked, decision("withdrawn", open, answer, pickup, withdrawn)]) {
    assert.equal(draw(c).includes('data-write="withdraw"'), false, c.state);
  }
});

test("a case that is not open shows no key hints", () => {
  assert.equal(draw(decision("pickedup", open, answer, pickup)).includes("key-hint"), false);
});

test("the option the agent recommends is marked in the form, and named in the properties", () => {
  const page = draw({ ...decision("open", open), recommend: 2 });
  assert.deepEqual(markedIn(page, ">recommended<"), ["2"]);
  // The mark flows with the option's text, and the key hint stays at the
  // row's end.
  const row = page.slice(at(page, 'value="2"'), page.indexOf("</label>", at(page, 'value="2"')));
  assert.ok(at(row, "a weekend away") < at(row, ">recommended<"));
  assert.ok(at(row, ">recommended<") < at(row, 'class="key-hint"'));
  assert.ok(page.includes('<div class="prop-recommends named"><dt>recommends</dt><dd><span class="recommends" title="2. a weekend away">2. a weekend away</span></dd></div>'));
  // A case that recommends none has neither.
  const none = draw(decision("open", open));
  assert.deepEqual(markedIn(none, ">recommended<"), []);
  assert.equal(none.includes("prop-recommends"), false);
});

test("a follow-up marks the recommended option and the last pick apart", () => {
  const chose = { ...answer, data: { choice: 3 } };
  const page = draw({ ...decision("open", open, chose, pickup, note), recommend: 1 });
  assert.deepEqual(markedIn(page, ">recommended<"), ["1"]);
  assert.deepEqual(markedIn(page, ">your last pick<"), ["3"]);
  // One option can carry both.
  const same = draw({ ...decision("open", open, chose, pickup, note), recommend: 3 });
  assert.deepEqual(markedIn(same, ">recommended<"), ["3"]);
  assert.deepEqual(markedIn(same, ">your last pick<"), ["3"]);
});

test("a follow-up stays one when the agent changes the recommend after its note", () => {
  // A new recommend leaves the amend seq alone (ST-04), so the case is not
  // asked afresh: the note leads, and the form marks both options.
  const chose = { ...answer, data: { choice: 3 } };
  const amend = { seq: 5, author: "agent", event: "amend", file: "", data: { recommend: 1 } };
  const page = draw({ ...decision("open", open, chose, pickup, note, amend), recommend: 1 });
  assert.ok(at(page, "new since your answer") < at(page, "Follow-up: a watch or a print, then?"));
  assert.deepEqual(markedIn(page, ">recommended<"), ["1"]);
  assert.deepEqual(markedIn(page, ">your last pick<"), ["3"]);
  assert.ok(page.includes("recommended 1. a watch"));
});

test("the case's links are a list under the body and above the thread, each in full", () => {
  const address = "https://example.com/pr/1";
  const page = draw({ ...decision("open", open), links: [address, "src/main.rs"] });
  const links = at(page, '<section class="case-links"');
  assert.ok(at(page, body) < links);
  // In the body's article, so the thread's grid row, beside a wide form,
  // does not open between the links and the thread.
  assert.ok(links < at(page, "</article>"));
  assert.ok(links < at(page, '<ol class="thread">'));
  // Only an address the page may link to is a link; a path is text.
  assert.ok(page.includes(`<li><a href="${address}" target="_blank" rel="noopener noreferrer">${address}</a></li>`));
  assert.ok(page.includes("<li>src/main.rs</li>"));
  assert.equal(draw(decision("open", open)).includes("case-links"), false);
});

test("the thread says which option an amend recommends, when it changes", () => {
  const amend = (seq, data) => ({ seq, author: "agent", event: "amend", file: "", data });
  const c = { ...decision("open", open, amend(2, { recommend: 3 }), amend(3, { recommend: 3, links: ["https://example.com/pr"] })), recommend: 3 };
  const page = draw(c);
  assert.equal(page.match(/recommended 3\. a print/g)?.length, 1);
  // It starts from the open's recommend, and an amend without one says
  // nothing of it.
  const recommended = { ...open, data: { ...open.data, recommend: 2 } };
  const from = draw({ ...decision("open", recommended, amend(2, { recommend: 2, links: ["https://example.com/pr"] }), amend(3, { labels: ["later"] })), recommend: 2 });
  assert.equal(from.includes("recommended 2."), false);
  assert.equal(/recommended (0|undefined)/.test(from), false);
});

test("a row's verdict is the one chosen, else approve where the options pre-select it, else none", () => {
  assert.equal(verdictOf({}, "r1", false), undefined);
  assert.equal(verdictOf({}, "r1", true), "approve");
  assert.equal(verdictOf({ r1: "hold" }, "r1", true), "hold");
  assert.equal(verdictOf({ r1: "reject" }, "r1", false), "reject");
  // A row an amend adds has no verdict chosen, so it starts pre-selected.
  assert.equal(verdictOf({ r1: "hold" }, "r2", true), "approve");
  // Only the verdicts' own properties count.
  assert.equal(verdictOf({}, "constructor", false), undefined);
});

test("with approve pre-selected, every row of an open approval starts approved and marked as only pre-selected", () => {
  const rows = ["r1", "r2"].map((id) => ({ id, label: `row ${id}`, link: "", script: "true" }));
  const approval = { ...decision("open", open), kind: "approval", options: undefined, rows };
  const checked = (page) => [...page.matchAll(/<input type="radio" name="verdict\.(\w+)" value="(\w+)"[^>]*checked/g)].map((m) => `${m[1]}=${m[2]}`);
  assert.deepEqual(checked(draw(approval)), []);
  assert.equal(draw(approval).includes("data-preset"), false);
  globalThis.window = { casesPrefs: { read: (name) => (name === "approve" ? "on" : "") } };
  try {
    const page = draw(approval);
    assert.deepEqual(checked(page), ["r1=approve", "r2=approve"]);
    assert.equal(page.match(/<fieldset class="row" data-preset="true">/g)?.length, 2, page);
  } finally {
    delete globalThis.window;
  }
});

// An approval with a row the agent runs, r1, and one the human carries
// out, r2 (case i2lDf).
function withHumanRow() {
  const rows = [
    { id: "r1", label: "Install deps", link: "", script: "npm ci" },
    { id: "r2", label: "Point DNS", link: "", script: "Point the name at the new host", by: "human" },
  ];
  return { ...decision("open", open), kind: "approval", options: undefined, rows };
}

// The words each verdict of row `id` in `page` shows, by its value.
function verdictWordsIn(page, id) {
  const choices = page.match(new RegExp(`<label class="choice"><input type="radio" name="verdict\\.${id}" (?:(?!</label>).)*</label>`, "g")) || [];
  return choices.map((choice) => [/value="(\w+)"/.exec(choice)[1], /\/> (.*?) <span class="hint">/.exec(choice)[1]]);
}

test("a row the human carries out is tagged as theirs, and its approve reads done, still sent as approve", () => {
  const page = draw(withHumanRow());
  assert.ok(page.includes('<fieldset class="row" data-by="human">'), page);
  assert.ok(page.includes('<legend class="row-title">Point DNS <span class="badge by-human" title="a step you carry out">you</span></legend>'), page);
  assert.deepEqual(verdictWordsIn(page, "r2"), [["approve", "done"], ["hold", "hold"], ["reject", "won't do"]]);
  // A row the agent runs reads as it did, with no tag.
  assert.ok(page.includes('<fieldset class="row"><legend class="row-title">Install deps</legend>'), page);
  assert.deepEqual(verdictWordsIn(page, "r1"), [["approve", "approve"], ["hold", "hold"], ["reject", "don't run"]]);
  assert.equal(page.match(/badge by-human/g)?.length, 1);
  // The keys give the human's row its verdicts as they do any row's.
  assert.deepEqual(numbered(page), [["approve", "1"], ["hold", "2"], ["reject", "3"], ["approve", "1"], ["hold", "2"], ["reject", "3"]]);
});

test("with approve pre-selected, a row the human carries out starts with no verdict: done is the human's to say", () => {
  const checked = (page) => [...page.matchAll(/<input type="radio" name="verdict\.(\w+)" value="(\w+)"[^>]*checked/g)].map((m) => `${m[1]}=${m[2]}`);
  globalThis.window = { casesPrefs: { read: (name) => (name === "approve" ? "on" : "") } };
  try {
    const page = draw(withHumanRow());
    assert.deepEqual(checked(page), ["r1=approve"]);
    assert.ok(page.includes('<fieldset class="row" data-by="human"><legend'), page);
  } finally {
    delete globalThis.window;
  }
});

test("the note to the agent shows the n key that goes to it, which the field names, and the properties list it", () => {
  for (const [kind, name] of [["decision", "note to the agent"], ["signoff", "comment"], ["fyi", "note to the agent"]]) {
    const page = draw({ ...decision("open", open), kind });
    assert.ok(page.includes(`<label>${name} <span class="key-hint" aria-hidden="true"><kbd>n</kbd></span> <textarea name="note" rows="3" aria-keyshortcuts="n"></textarea></label>`), kind);
    assert.ok(page.includes("<dt>note</dt><dd><kbd>n</kbd></dd>"), kind);
  }
  // A case not open has no form, and no n.
  assert.equal(draw(decision("pickedup", open, answer, pickup)).includes("<dt>note</dt>"), false);
});

// A message the human sent to bun-pins, then picked up and closed as it is
// after each step.
const human = { id: "h1", kind: "human", name: "ryan" };
const agent = { id: "a2", kind: "agent", name: "bun-pins" };
const sentEvent = { seq: 1, author: "human", event: "open", file: "", at: "2026-09-25T13:10:00Z", actor: human, data: { kind: "message", worker: "bun-pins", body: "Rebase onto main." } };
const pickupEvent = { seq: 2, author: "agent", event: "pickup", from: "sent", file: "", at: "2026-09-25T13:40:00Z", actor: agent, data: { by: "bun-pins" } };
const closeEvent = { seq: 3, author: "agent", event: "close", from: "pickedup", file: "", at: "2026-09-25T13:44:00Z", actor: agent, data: { outcome: "Rebased in abc123." } };
function message(state, ...events) {
  const c = { id: "mS1gX", state, kind: "message", urgency: "today", title: "Rebase onto main", body: "Rebase onto main.", worker: "bun-pins", actor: human, opened_at: sentEvent.at, events, revision: events.length, amend_seq: 0, last_seq: Math.max(0, ...events.map((e) => e.seq)) };
  if (events.includes(pickupEvent)) c.pickup = { by: "bun-pins", picked_up_at: pickupEvent.at };
  if (events.includes(closeEvent)) c.close = { outcome: "Rebased in abc123.", closed_at: closeEvent.at };
  return c;
}
const drawWith = (c, props = {}) => render(html`<${CaseView} c=${c} now=${now} onSent=${() => {}} onCase=${() => {}} ...${props} />`);

test("a sent message says who it went to, and takes an edit, folded, and a withdraw, not folded", () => {
  const page = drawWith(message("sent", sentEvent));
  assert.ok(page.includes('<p class="to">message to <strong>bun-pins</strong></p>'), page);
  assert.ok(page.includes('data-write="amend"') && page.includes('<details class="edit">'), page);
  assert.ok(page.includes('<form class="respond withdraw-sent" data-write="withdraw">'), page);
  assert.ok(!page.includes('<details class="withdraw">'), page);
  assert.ok(!page.includes('data-write="answer"'), page);
  assert.ok(!page.includes("message-agent"), "a message offers no message this agent");
  // Who sent it and when are its properties: a message has no thread.
  assert.ok(page.includes('<dt>from</dt><dd>ryan</dd>') && page.includes("<dt>opened</dt>"), page);
});

test("what comes before the answer box and what comes after it are one block each", () => {
  // Beside a wide form, the second block grows to the form's height, so the
  // withdraw, the cases about the message and the thread all go in it, and
  // none is pushed below the form.
  const about = { id: "fU1pq", state: "open", kind: "fyi", urgency: "today", title: "About the rebase", re: "mS1gX", events: [], revision: 1, amend_seq: 0, last_seq: 1 };
  const page = drawWith(message("sent", sentEvent), { cases: [about] });
  const before = at(page, '<div class="before-answer">');
  const box = at(page, '<div class="answer">');
  const after = at(page, '<div class="after-answer">');
  assert.ok(before < at(page, "Rebase onto main.</p>") && at(page, "Rebase onto main.</p>") < box);
  assert.ok(box < at(page, 'data-write="amend"') && at(page, 'data-write="amend"') < after);
  for (const part of ['data-write="withdraw"', '<section class="follow-ups"']) assert.ok(after < at(page, part), part);
  // A closed message's outcome and who took it come before the box.
  const closed = draw(message("closed", sentEvent, pickupEvent, closeEvent));
  for (const part of ['<section class="outcome"', '<p class="taken">']) {
    assert.ok(at(closed, '<div class="before-answer">') < at(closed, part) && at(closed, part) < at(closed, '<div class="answer">'), part);
  }
});

test("a picked-up message says who has it and what they say they are doing, and folds its withdraw", () => {
  const listeners = [{ worker: "bun-pins", labels: [], since: "2026-09-25T13:00:00Z", heard: "2026-09-25T13:44:55Z", doing: "rebasing", doing_at: "2026-09-25T13:41:00Z" }];
  const page = drawWith(message("pickedup", sentEvent, pickupEvent), { listeners });
  assert.ok(page.includes('<p class="taken">picked up by <strong>bun-pins</strong>, 5m ago<span class="doing">doing: rebasing · 4m ago</span></p>'), page);
  assert.ok(page.includes('<details class="withdraw">'), page);
  assert.ok(page.includes("<summary>withdraw this message</summary>"), page);
  assert.ok(!page.includes("the answer stays in the thread"), page);
  assert.ok(!page.includes('data-write="amend"'), page);
});

test("a closed message shows its outcome, with its links, and who took it", () => {
  const closed = message("closed", sentEvent, pickupEvent, closeEvent);
  closed.close.links = ["https://example.com/pr/1"];
  const page = drawWith(closed);
  assert.ok(page.includes('<section class="outcome"') && page.includes("Rebased in abc123."), page);
  assert.ok(page.includes('href="https://example.com/pr/1"'), page);
  assert.ok(page.includes('<p class="taken">picked up by <strong>bun-pins</strong>'), page);
  assert.ok(!page.includes('data-write="withdraw"'), page);
});

// A message's thread is only its send and what the agent did, which its
// page says elsewhere (case:QeNA4): it has none, in any state.
test("a message's page has no thread, and says elsewhere what the thread did", () => {
  const withdrawEvent = { seq: 3, author: "human", event: "withdraw", file: "", at: "2026-09-25T13:50:00Z", actor: human, data: { reason: "Not needed now." } };
  const amendEvent = { seq: 2, author: "human", event: "amend", file: "", at: "2026-09-25T13:20:00Z", actor: human, data: { body: "Rebase onto main, then test." } };
  const states = {
    sent: message("sent", sentEvent, amendEvent),
    pickedup: message("pickedup", sentEvent, pickupEvent),
    closed: message("closed", sentEvent, pickupEvent, closeEvent),
    withdrawn: message("withdrawn", sentEvent, pickupEvent, withdrawEvent),
  };
  for (const [state, c] of Object.entries(states)) {
    const page = drawWith(c);
    assert.ok(!page.includes('id="thread"'), state);
    assert.ok(page.includes('<dt>from</dt><dd>ryan</dd>') && page.includes("<dt>opened</dt>"), state);
  }
  // An edit before pickup is its properties' "amended".
  assert.ok(drawWith(states.sent).includes("<dt>amended</dt>"));
  // A withdraw says who, when and why, where the thread did.
  const withdrawn = drawWith(states.withdrawn);
  assert.ok(withdrawn.includes('<p class="taken withdrawn">withdrawn by <strong>ryan</strong>, ') && withdrawn.includes('<span class="reason">Not needed now.</span>'), withdrawn);
  assert.ok(withdrawn.includes('<p class="taken">picked up by <strong>bun-pins</strong>'), withdrawn);
  // A case of any other kind keeps its thread.
  assert.ok(drawWith(decision("open", steps.open)).includes('<section id="thread">'));
});

test("a case says what it is about and lists the cases about it, and offers to message its agent", () => {
  const question = { ...decision("open", steps.open), id: "qU3st", worker: "gift-agent", re: "mS1gX" };
  const theMessage = message("pickedup", sentEvent, pickupEvent);
  const page = drawWith(question, { cases: [theMessage] });
  assert.ok(page.includes('<p class="re">about <a href="/cases/mS1gX-rebase-onto-main">Rebase onto main</a></p>'), page);
  assert.ok(page.includes('<p class="message-agent"><a href="/message?to=gift-agent&amp;re=qU3st">message gift-agent</a></p>'), page);
  const about = drawWith(theMessage, { cases: [question] });
  assert.ok(about.includes('<h2 class="section" id="follow-ups-head">about this message</h2>'), about);
  assert.ok(about.includes('<a href="/cases/qU3st-which-way-for-the-anniversary-present">Which way for the anniversary present?</a><span class="flight">decision · open</span>'), about);
  // A case about one the page did not read, such as one pruned, names it by id.
  assert.ok(drawWith(question).includes('<p class="re">about <a href="/cases/mS1gX">mS1gX</a></p>'));
});

test("a form keeps its revision after the agent's amend that changes the question, and takes it after a note or a message's edit", () => {
  const was = drawnFrom(decision("open", open));
  const amend = { seq: 2, author: "agent", event: "amend", from: "open", file: "", at: open.at, data: { body: "Which way now?" } };
  const agentNote = { ...note, seq: 2, from: "open" };
  // The agent's amend of the body changes the question: the form's next
  // send is refused as stale (IN-06).
  assert.deepEqual(redrawn(was, { ...decision("open", open, amend), amend_seq: 2 }), was);
  assert.equal(redrawn(was, decision("open", open, agentNote)).revision, 2);
  // A change of state draws the form afresh.
  assert.equal(redrawn(was, decision("answered", open, answer)).revision, 2);
  // A message's edit is the human's own: its forms go on at the new revision.
  const sent = message("sent", sentEvent);
  const edit = { seq: 2, author: "human", event: "amend", from: "sent", file: "", at: sentEvent.at, actor: human, data: { body: "Rebase, then test." } };
  const edited = { ...message("sent", sentEvent, edit), body: "Rebase, then test.", amend_seq: 2 };
  assert.deepEqual(redrawn(drawnFrom(sent), edited), drawnFrom(edited));
});

test("a withdraw drawn on an answered case is refused after a reopen and a new answer between two reads, and drawn afresh when it moved on to a pickup", () => {
  // Drawn on the case answered; then, before the page reads again, the agent
  // reopens it with a note and it is answered again elsewhere.
  const answered = decision("answered", open, answer);
  const was = drawnFrom(answered);
  const reopen = { ...note, seq: 3, from: "answered", data: { ...note.data, reopen: true } };
  const again = { ...answer, seq: 4, data: { choice: 3 } };
  // In the state it was drawn in, by way of another: the form keeps its
  // revision, so that the withdraw is refused as stale (IN-05).
  assert.deepEqual(redrawn(was, decision("answered", open, answer, reopen, again)), was);
  // Picked up as well, the case is in another state: the withdraw is keyed by
  // the state and drawn afresh, folded and empty, under the new answer, so
  // the human withdraws only what the page now shows (IN-06).
  const picked = { ...pickup, seq: 5 };
  const now = decision("pickedup", open, answer, reopen, again, picked);
  assert.deepEqual(redrawn(was, now), drawnFrom(now));
  const page = draw(now);
  assert.ok(page.includes('<details class="withdraw">'), page);
  assert.ok(!page.includes('<details class="withdraw" open'), page);
});

test("a message's page says what the worker that has it is doing only while it has it, found by its worker when it was taken by another name", () => {
  const listeners = [{ worker: "bun-pins", labels: [], since: "2026-09-25T13:00:00Z", heard: "2026-09-25T13:44:55Z", doing: "rebasing", doing_at: "2026-09-25T13:41:00Z" }];
  // Taken by a wait whose --by is not its --worker.
  const taken = { ...message("pickedup", sentEvent, pickupEvent), pickup: { by: "sess-7", picked_up_at: pickupEvent.at } };
  const page = drawWith(taken, { listeners });
  assert.ok(page.includes('picked up by <strong>sess-7</strong>, 5m ago<span class="doing">doing: rebasing · 4m ago</span>'), page);
  // Closed, it is no longer what the worker is doing.
  const closed = drawWith(message("closed", sentEvent, pickupEvent, closeEvent), { listeners });
  assert.ok(closed.includes('<p class="taken">picked up by <strong>bun-pins</strong>, 5m ago</p>'), closed);
  assert.ok(!closed.includes("doing:"), closed);
});

test("a message's edit is drawn holding its body, which is not typed text until it differs", () => {
  const page = drawWith(message("sent", sentEvent));
  assert.ok(page.includes('<form class="respond" data-write="amend"><details class="edit">'), page);
  assert.ok(page.includes(">Rebase onto main.</textarea>"), page);
  assert.ok(!page.includes("data-edited"), page);
});

test("an open or amend whose body is the case's (body_in_case) is read from the case, so the thread still says what each amend replaced", () => {
  // Amended from A to B and back: the open and the last amend set the body
  // the case has, and leave it out of their records.
  const at = (m) => `2026-09-25T13:${m}:00Z`;
  const agent = { id: "a1", kind: "agent", name: "gift-agent" };
  const events = [
    { seq: 1, author: "agent", event: "open", file: "", at: at(10), actor: agent, data: { title: "T" }, body_in_case: true },
    { seq: 2, author: "agent", event: "amend", file: "", at: at(11), actor: agent, data: { body: "B" } },
    { seq: 3, author: "agent", event: "amend", file: "", at: at(12), actor: agent, data: {}, body_in_case: true },
  ];
  const c = { ...decision("open"), body: "A", events };
  const said = thread(c).map(({ lines }) => lines.filter((l) => l.text === "replaced the body").map((l) => l.previous));
  assert.deepEqual(said, [[], [{ body: "A" }], [{ body: "B" }]]);
});

// An event whose record has no data reads as one with none of its fields,
// as the thread reads it, and leaves the page drawn.
test("a follow-up whose pickup or note has no data still draws, naming the pickup's actor", () => {
  const page = draw(decision("open", open, answer, { ...pickup, data: null }, note));
  assert.ok(at(page, "your previous answer") < at(page, "picked up by gift-agent"), page);
  const blank = { ...note, data: null };
  const noted = draw(decision("open", open, answer, pickup, blank));
  assert.ok(at(noted, "new since your answer") < at(noted, "your previous answer"), noted);
});

test("a withdrawn message whose withdraw has no data says who withdrew it, and no reason", () => {
  const withdrawEvent = { seq: 3, author: "human", event: "withdraw", file: "", at: "2026-09-25T13:50:00Z", actor: human, data: null };
  const page = drawWith(message("withdrawn", sentEvent, pickupEvent, withdrawEvent));
  assert.ok(page.includes('<p class="taken withdrawn">withdrawn by <strong>ryan</strong>, '), page);
  assert.ok(!page.includes('class="reason"'), page);
});

// A form's fields, with `extra` set.
const fields = (extra) => ({ choice: "", verdicts: {}, rowNotes: {}, signoff: "", briefing: "", file: [], stuck: "", text: "", note: "", ...extra });

test("the form sends other only with a note, as the store wants (IN-03)", () => {
  const c = decision("open", open);
  for (const note of ["", "   "]) {
    assert.deepEqual(response(c, fields({ choice: "other", note }), false), { problem: "other needs a note: say what you want instead" });
  }
  assert.deepEqual(response(c, fields({ choice: "other", note: " a print " }), false), { answer: { note: "a print", other: true } });
  assert.deepEqual(response(c, fields({ choice: "1" }), false), { answer: { choice: 1 } });
});

test("the form sends request changes only with a comment, as the store wants (IN-03)", () => {
  const c = { ...decision("open", open), kind: "signoff", options: undefined };
  for (const note of ["", "   "]) {
    assert.deepEqual(response(c, fields({ signoff: "changes", note }), false), { problem: "requesting changes needs a comment: say what to change" });
  }
  assert.deepEqual(response(c, fields({ signoff: "changes", note: "rename it" }), false), { answer: { note: "rename it", signoff: "changes" } });
  assert.deepEqual(response(c, fields({ signoff: "accept" }), false), { answer: { signoff: "accept" } });
});
