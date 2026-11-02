// What CaseView draws for each kind and state of case, and what its sends
// do: the write it posts, the case it hands on, and the refusal it shows.
import { test } from "node:test";
import assert from "node:assert/strict";
import { options } from "preact";
import { render } from "preact-render-to-string";
import { html } from "../static/html.ts";
import { CaseView } from "../static/case.ts";
import { decision, steps } from "./case.js";

const now = Date.parse("2026-09-25T13:45:00Z");
const opened = { seq: 1, author: "agent", event: "open", file: "", at: "2026-09-25T13:10:00Z", data: {} };

// A case of `kind` in `state`, with `fields` over the rest.
function make(kind, state, fields = {}) {
  return { id: "c1Ab2", kind, state, urgency: "today", title: "The title", body: "The body.", options: ["one", "two"], opened_at: opened.at, events: [opened], revision: 3, amend_seq: 0, last_seq: 1, ...fields };
}
const draw = (c, props = {}) => render(html`<${CaseView} c=${c} now=${now} onSent=${() => {}} onCase=${() => {}} ...${props} />`);
// Whether `page` says `text`, for each case in `cases`.
const says = (text, cases, props) => cases.map((c) => draw(c, props).includes(text));

test("the head says a briefing's landing, a message's addressee, what the case is about, its context and brief", () => {
  assert.deepEqual(says('<p class="landed-mark landed">landed</p>', [make("briefing", "open", { landed: true }), make("briefing", "open"), make("decision", "open", { landed: true })]), [true, false, false]);
  assert.deepEqual(says('<p class="landed-mark gated">waiting to land</p>', [make("briefing", "open"), make("briefing", "open", { landed: true })]), [true, false]);
  assert.deepEqual(says('<p class="to">message to <strong>w1</strong></p>', [make("message", "sent", { worker: "w1" }), make("decision", "open", { worker: "w1" })]), [true, false]);
  const re = make("decision", "open", { re: "zZ9yY" });
  assert.ok(draw(re).includes('<p class="re">about <a href="/cases/zZ9yY">zZ9yY</a></p>'), draw(re));
  assert.ok(!draw(make("decision", "open")).includes('class="re"'));
  assert.deepEqual(says('<p class="context">the context</p>', [make("decision", "open", { context: "the context" }), make("decision", "open")]), [true, false]);
  assert.deepEqual(says('<p class="brief"><span class="label">brief</span> the brief</p>', [make("decision", "open", { brief: "the brief" }), make("decision", "open")]), [true, false]);
  // A case an agent sent has a link to message it; a message, or a case with
  // no worker, has none.
  assert.deepEqual(says('<p class="message-agent"><a href=', [make("decision", "open", { worker: "w1" }), make("message", "sent", { worker: "w1" }), make("decision", "open")]), [true, false, false]);
});

test("the article holds the body, or the follow-up, then the left-out items and the links", () => {
  assert.deepEqual(says("<p>The body.</p>", [make("decision", "open"), decision("open", steps.open, steps.answer, steps.pickup, steps.note)]), [true, false]);
  assert.deepEqual(says('<section class="follow-up"', [make("decision", "open"), decision("open", steps.open, steps.answer, steps.pickup, steps.note)]), [false, true]);
  assert.deepEqual(says('<section class="left-out" aria-labelledby="left-out-head"><h2 class="section" id="left-out-head">left out</h2><ol><li>a</li><li>b</li></ol></section>', [make("briefing", "open", { left_out: ["a", "b"] }), make("briefing", "open", { left_out: [] }), make("briefing", "open")]), [true, false, false]);
  assert.deepEqual(says('<section class="case-links" aria-labelledby="links-head"><h2 class="section" id="links-head">links</h2><ul class="links"><li>', [make("decision", "open", { links: ["https://example.com/a"] }), make("decision", "open", { links: [] }), make("decision", "open")]), [true, false, false]);
});

test("a closed message says its outcome, with its links when it has some", () => {
  const close = (links) => ({ outcome: "**Done**.", closed_at: opened.at, ...(links && { links }) });
  const outcome = '<section class="outcome" aria-labelledby="outcome-head"><h2 class="section" id="outcome-head">outcome</h2><div class="body"><p><strong>Done</strong>.</p>\n</div>';
  assert.deepEqual(says(outcome, [make("message", "closed", { close: close() }), make("message", "closed"), make("decision", "closed", { close: close() }), make("message", "pickedup", { close: close() })]), [true, false, false, false]);
  const page = draw(make("message", "closed", { close: close(["https://example.com/pr"]) }));
  assert.ok(page.includes(`${outcome}<ul class="links"><li>`), page);
  assert.ok(!draw(make("message", "closed", { close: close([]) })).includes(`${outcome}<ul`));
});

test("a message says who picked it up, and who withdrew it", () => {
  const pickup = { by: "w1", picked_up_at: opened.at };
  assert.deepEqual(says('<p class="taken">picked up by <strong>w1</strong>', [make("message", "pickedup", { pickup }), make("message", "sent"), make("decision", "pickedup", { pickup })]), [true, false, false]);
  assert.deepEqual(says('<p class="taken withdrawn">withdrawn', [make("message", "withdrawn"), make("decision", "withdrawn"), make("message", "closed")]), [true, false, false]);
});

test("each state has its form: answer, resume, edit, withdraw, or none", () => {
  const forms = (c) => ["answer", "resume", "amend", "withdraw"].filter((w) => draw(c).includes(`data-write="${w}"`));
  assert.deepEqual(forms(make("decision", "open")), ["answer"]);
  assert.deepEqual(forms(make("decision", "parked")), ["resume"]);
  assert.deepEqual(forms(make("message", "sent")), ["amend", "withdraw"]);
  assert.deepEqual(forms(make("decision", "answered")), ["withdraw"]);
  assert.deepEqual(forms(make("decision", "pickedup")), ["withdraw"]);
  assert.deepEqual(forms(make("decision", "closed")), []);
  assert.deepEqual(forms(make("decision", "withdrawn")), []);
  // The refusal line is there in every state, hidden while there is none.
  assert.ok(draw(make("decision", "closed")).includes('<p class="error" role="alert" id="refusal" hidden></p>'));
});

test("after the answer: the cases about this one, and the thread, which a message has not", () => {
  const about = [make("question", "open", { id: "q1", title: "Asked", re: "c1Ab2" }), make("question", "open", { id: "q2", title: "Other", re: "x" })];
  const page = draw(make("decision", "open"), { cases: about });
  assert.ok(page.includes('<h2 class="section" id="follow-ups-head">about this case</h2>') && page.includes(">Asked</a>") && !page.includes(">Other</a>"), page);
  assert.ok(draw(make("message", "sent"), { cases: about }).includes(">about this message</h2>"));
  assert.ok(!draw(make("decision", "open")).includes("follow-ups-head"));
  assert.deepEqual(says('<section id="thread">', [make("decision", "open"), make("message", "sent")]), [true, false]);
});

// The props of each element drawn while `draw` runs that `pick` picks.
function drawn(pick, draw) {
  const was = options.vnode;
  const found = [];
  options.vnode = (vnode) => {
    if (vnode.props && pick(vnode.props)) found.push(vnode.props);
    if (was) was(vnode);
  };
  try {
    draw();
  } finally {
    options.vnode = was;
  }
  return found;
}

// Stands in for serve: the token at /v1, and `reply` to each POST, logged
// in `posts` as [path, body].
function serve(t, reply) {
  const was = globalThis.fetch;
  t.after(() => {
    globalThis.fetch = was;
  });
  const posts = [];
  globalThis.fetch = async (url, init = {}) => {
    if (url === "/v1") return new Response(JSON.stringify({ csrf: "tok" }), { status: 200 });
    posts.push([url, JSON.parse(init.body)]);
    return reply(url, init);
  };
  return posts;
}

// The `send` CaseView gives its forms for case `c`, with what it told
// `onSent` and `onCase` logged in `told`.
function sender(c) {
  const told = [];
  const [props] = drawn((p) => typeof p.send === "function" && typeof p.refuse === "function", () => draw(c, { onSent: (what, after) => told.push(["sent", what, after.revision]), onCase: (current) => told.push(["case", current.revision]) }));
  return { send: props.send, told };
}

test("an answer is sent with the revision and amend seq it was drawn at, and the case after it is handed on", async (t) => {
  const posts = serve(t, () => new Response(JSON.stringify({ ...make("decision", "answered"), revision: 4 }), { status: 200 }));
  const { send, told } = sender(make("decision", "open", { amend_seq: 2 }));
  await send("answer", { answer: { choice: 1 } });
  assert.deepEqual(posts, [["/v1/cases/c1Ab2/answer", { revision: 3, answer: { choice: 1, amend_seq: 2 } }]]);
  assert.deepEqual(told, [["sent", "answer", 4]]);
  // Another write takes no amend seq.
  await send("withdraw", { reason: "r" });
  assert.deepEqual(posts[1], ["/v1/cases/c1Ab2/withdraw", { revision: 3, reason: "r" }]);
});

test("a refused send hands on the case as it is now, when serve gives it, and nothing else", async (t) => {
  const stale = { error: { category: "stale", message: "stale", case: { ...make("decision", "open"), revision: 7 } } };
  let reply = stale;
  serve(t, () => new Response(JSON.stringify(reply), { status: 409 }));
  const { send, told } = sender(make("decision", "open"));
  await send("answer", { answer: { choice: 1 } });
  assert.deepEqual(told, [["case", 7]]);
  reply = { error: { category: "store", message: "cannot write" } };
  await send("answer", { answer: { choice: 1 } });
  assert.deepEqual(told, [["case", 7]]);
});

test("a send while one is out does nothing, and the next after it is sent", async (t) => {
  let answer;
  // The first post waits until the test answers it; the others are answered
  // at once.
  const parked = () => new Response(JSON.stringify(make("decision", "parked")), { status: 200 });
  const posts = serve(t, () => (answer ? parked() : new Promise((resolve) => (answer = resolve))));
  const { send } = sender(make("decision", "open"));
  const first = send("park", {});
  await send("park", {});
  while (!answer) await new Promise((r) => setTimeout(r, 1));
  assert.equal(posts.length, 1);
  answer(parked());
  await first;
  await send("park", {});
  assert.equal(posts.length, 2);
});

test("a send that cannot reach serve hands nothing on, and the next is sent", async (t) => {
  let fail = true;
  const posts = [];
  const was = globalThis.fetch;
  t.after(() => {
    globalThis.fetch = was;
  });
  globalThis.fetch = async (url, init) => {
    if (url === "/v1") return new Response(JSON.stringify({ csrf: "tok" }), { status: 200 });
    posts.push(url);
    if (fail) throw new TypeError("network");
    return new Response(JSON.stringify(make("decision", "parked")), { status: 200 });
  };
  const { send, told } = sender(make("decision", "open"));
  await send("park", {});
  assert.deepEqual(told, []);
  fail = false;
  await send("park", {});
  assert.deepEqual(told, [["sent", "park", 3]]);
  assert.equal(posts.length, 2);
});

test("a parked case's resume form sends a resume", async (t) => {
  const posts = serve(t, () => new Response(JSON.stringify(make("decision", "open")), { status: 200 }));
  const [form] = drawn((p) => p["data-write"] === "resume", () => draw(make("decision", "parked")));
  let prevented = false;
  form.onSubmit({ preventDefault: () => (prevented = true) });
  while (!posts.length) await new Promise((r) => setTimeout(r, 1));
  assert.ok(prevented);
  assert.deepEqual(posts, [["/v1/cases/c1Ab2/resume", { revision: 3 }]]);
});
